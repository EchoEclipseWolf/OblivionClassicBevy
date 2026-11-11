//! NiPropertyState and NiDynamicEffectState, and how they reach geometry: 00707610 RefreshPropertyState and 00707980
//! RefreshEffectState build the inherited state of the object's parent chain and push it down through vtable +0x6C /
//! +0x70. NiNode merges its own lists (0070A840, 0070A900), NiGeometry installs the result at +0xAC / +0xB0 (00722B30,
//! 00722C00) and every other reached class has the 0068F970 no-op. No rendering: later phases read the states.
//!
//! A state is an entity, because its identity is observable: a merge can return the inherited state itself, and +0xAC is
//! replaced only when it differs. Host substitution: the 00B3FA00 critical section is the exclusive `&mut World` borrow.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::av_object::{NiAvParent, NiAvProperties};
use super::mesh_geometry::NiGeometry;
use super::object_net::AlphaProperty;
use super::particle_system::ParticleSystem;
use super::properties::{
    FogProperty, MaterialProperty, SkyShaderProperty, StencilProperty, VertexColorProperty,
    WaterShaderProperty, WireframeProperty, ZBufferProperty,
};
use super::registration::{NiFogProperty, NiRendererSpecificProperty, NiShadeProperty};
use super::scene_derived::DirectionalLight;
use super::scene_node::{NiNodeChildren, NiNodeEffects};
use super::stream_objects::NiClass;
use crate::gamebryo::registration::NiProcessDefaults;

/// IDA: vtables of the two state classes.
pub const NI_PROPERTY_STATE_VTABLE: u32 = 0x00A8_0984;
pub const NI_DYNAMIC_EFFECT_STATE_VTABLE: u32 = 0x00A7_E350;

/// IDA: 007319E0 NiPropertyState (0x30 bytes, NiRefObject base): ten NiPointer<NiProperty> slots +0x08..+0x2C indexed by
/// the property type (alpha 0, fog 1, material 2, renderer-specific 3, shade 4, stencil 5, texturing 6, vertex color 7,
/// wireframe 8, z-buffer 9). Completion: 100% of the slot layout.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ScenePropertyState {
    pub original_vtable: u32,
    pub slots: [Option<Entity>; 10],
    /// IDA: N/A the installs on geometry, standing for the NiRefObject +0x04 count: a state nothing holds is despawned.
    pub host_references: u32,
}

/// IDA: 00709E60 / 00731B60 NiDynamicEffectState (0x20 bytes, NiRefObject base): +0x08 byte (any projected light,
/// projected shadow, fog map or environment map present), +0x0C light list, +0x10 projected lights, +0x14 projected
/// shadows (singly linked {next, effect} nodes kept sorted by the effect +0xB0 UID, 00731BF0), +0x18 fog map, +0x1C
/// environment map. Only NiDirectionalLight (effect type 1, list +0x0C) is bound.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct SceneEffectState {
    pub original_vtable: u32,
    pub multiple_maps_08: u8,
    pub lights_0c: Vec<Entity>,
    pub projected_lights_10: Vec<Entity>,
    pub projected_shadows_14: Vec<Entity>,
    pub fog_map_18: Option<Entity>,
    pub environment_map_1c: Option<Entity>,
    /// IDA: N/A as `ScenePropertyState::host_references`.
    pub host_references: u32,
}

impl Default for SceneEffectState {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: NI_DYNAMIC_EFFECT_STATE_VTABLE,
            multiple_maps_08: 0,
            lights_0c: Vec::new(),
            projected_lights_10: Vec::new(),
            projected_shadows_14: Vec::new(),
            fog_map_18: None,
            environment_map_1c: None,
            host_references: 0,
        }
    }
}

/// IDA: 007317B0's process defaults (00B3FCE4..), as property entities. `registration::NiProcessDefaults` owns them and
/// `scene_property_defaults` reads them; they stay an argument because a caller holds `&mut World` here. A null default
/// leaves its slot null.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScenePropertyDefaults {
    pub alpha: Option<Entity>,
    pub material: Option<Entity>,
    pub renderer_specific: Option<Entity>,
    pub shade: Option<Entity>,
    pub stencil: Option<Entity>,
    pub texturing: Option<Entity>,
    pub vertex_color: Option<Entity>,
    pub wireframe: Option<Entity>,
    pub zbuffer: Option<Entity>,
}

/// IDA: N/A the count both state classes keep.
trait HostCountedState: Component<Mutability = bevy_ecs::component::Mutable> {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn host_references(&mut self) -> &mut u32;
}

impl HostCountedState for ScenePropertyState {
    /// IDA: N/A `ScenePropertyState`'s side of `HostCountedState`.
    fn host_references(&mut self) -> &mut u32 {
        &mut self.host_references
    }
}

impl HostCountedState for SceneEffectState {
    /// IDA: N/A `SceneEffectState`'s side of `HostCountedState`.
    fn host_references(&mut self) -> &mut u32 {
        &mut self.host_references
    }
}

// IDA: N/A where the C++ `shared_ptr` of a temporary state would drop to zero: despawn it unless geometry holds it.
fn discard_unreferenced<T: HostCountedState>(world: &mut World, state: Option<Entity>) {
    let Some(state) = state else {
        return;
    };
    let Some(mut owner) = world.get_mut::<T>(state) else {
        return;
    };
    if *owner.host_references() == 0 {
        world.despawn(state);
    }
}

// IDA: N/A the NiPointer assignment on +0xAC / +0xB0: the new state is retained, the old one released. The caller has
// checked that they differ.
fn replace_reference<T: HostCountedState>(
    world: &mut World,
    old: Option<Entity>,
    new: Option<Entity>,
) {
    if let Some(mut owner) = new.and_then(|new| world.get_mut::<T>(new)) {
        *owner.host_references() += 1;
    }
    if let Some(mut owner) = old.and_then(|old| world.get_mut::<T>(old)) {
        *owner.host_references() -= 1;
    }
    discard_unreferenced::<T>(world, old);
}

// IDA: N/A the +0x1C parent of an object; a stored parent whose entity has gone is a host error (the original pointer is
// raw). Completion: 100%.
fn object_parent(world: &World, object: Entity) -> Result<Option<Entity>, GameError> {
    let parent = world.get::<NiAvParent>(object).and_then(|parent| parent.0);
    if let Some(parent) = parent
        && world.get_entity(parent).is_err()
    {
        return Err(GameError::unclassified(
            0,
            "NiAVObject +0x1C parent owner expired",
        ));
    }
    Ok(parent)
}

// IDA: N/A the +0x98 list of an object, head first. An object not linked yet has none.
fn local_properties(world: &World, object: Entity) -> Vec<Option<Entity>> {
    world
        .get::<NiAvProperties>(object)
        .map(|properties| properties.0.clone())
        .unwrap_or_default()
}

// IDA: vtable +0x4C GetType on the bound classes: alpha 0 (006F7070), fog 1 (0091FEA0), material 2 (008CF6B0),
// renderer-specific 3 (0073FD50), shade 4 (0073DAA0, also BSShaderProperty), stencil 5 (00924E30), texturing 6 (007AFE90),
// vertex color 7 (004059C0), wireframe 8 (00419750), z-buffer 9 (007EB240). Fog (BSFogProperty and NiFogProperty),
// renderer-specific, shade, wireframe and the shaders have no factory and no `NiClass`, so their component says what
// they are. A null property dereferences the vtable in the original; the host reports an error.
// Completion: 100% for the bound classes.
pub fn property_type(world: &World, property: Option<Entity>) -> Result<u32, GameError> {
    const NULL_PROPERTY: GameError =
        GameError::unclassified(0x00405680, "00405680: null NiProperty");
    let property = property.ok_or(NULL_PROPERTY)?;
    match world.get::<NiClass>(property) {
        Some(NiClass::AlphaProperty) => return Ok(0),
        Some(NiClass::MaterialProperty) => return Ok(2),
        Some(NiClass::PpLightingProperty | NiClass::Lighting30Property) => return Ok(4),
        Some(NiClass::StencilProperty) => return Ok(5),
        Some(NiClass::TexturingProperty) => return Ok(6),
        Some(NiClass::VertexColorProperty) => return Ok(7),
        Some(NiClass::ZBufferProperty) => return Ok(9),
        // 0073D5D0.
        Some(NiClass::SpecularProperty) => return Ok(12),
        _ => {}
    }
    let has = |present: bool, value: u32| present.then_some(value);
    has(world.get::<AlphaProperty>(property).is_some(), 0)
        .or_else(|| has(world.get::<FogProperty>(property).is_some(), 1))
        .or_else(|| has(world.get::<NiFogProperty>(property).is_some(), 1))
        .or_else(|| has(world.get::<MaterialProperty>(property).is_some(), 2))
        .or_else(|| {
            has(
                world.get::<NiRendererSpecificProperty>(property).is_some(),
                3,
            )
        })
        .or_else(|| has(world.get::<NiShadeProperty>(property).is_some(), 4))
        .or_else(|| has(world.get::<SkyShaderProperty>(property).is_some(), 4))
        .or_else(|| has(world.get::<WaterShaderProperty>(property).is_some(), 4))
        .or_else(|| has(world.get::<StencilProperty>(property).is_some(), 5))
        .or_else(|| has(world.get::<VertexColorProperty>(property).is_some(), 7))
        .or_else(|| has(world.get::<WireframeProperty>(property).is_some(), 8))
        .or_else(|| has(world.get::<ZBufferProperty>(property).is_some(), 9))
        .ok_or(NULL_PROPERTY)
}

// IDA: 007077D0 NiAVObject_MergePropertyListIntoState's slot index: the property type ID returned by vtable +0x4C; a type
// above 10 (007078B8 `cmp eax, 0Ah; jg`) is skipped, `None`. Completion: 100%.
fn slot_type(world: &World, property: Entity) -> Result<Option<usize>, GameError> {
    let kind = property_type(world, Some(property))?;
    if kind > 10 {
        return Ok(None);
    }
    if kind == 10 {
        return Err(GameError::unclassified(
            0x007077D0,
            "007077D0: type 10 writes past the 0x30-byte NiPropertyState",
        ));
    }
    Ok(Some(kind as usize))
}

// IDA: 00405680 NiAVObject_AttachProperty on any bound NiAVObject class: a vtable +0x4C type below 10 AddHeads the
// property on +0x98 (00749800). Returns false otherwise. Completion: 100%.
pub fn attach_object_property(
    world: &mut World,
    object: Entity,
    property: Option<Entity>,
) -> Result<bool, GameError> {
    if !world
        .get::<NiClass>(object)
        .is_some_and(|class| class.is_av_object())
    {
        return Err(GameError::unclassified(
            0x00405680,
            "00405680: no mutable host property list",
        ));
    }
    // 004056B5 vtable +0x4C (a null property dereferences the vtable).
    if property_type(world, property)? >= 10 {
        return Ok(false);
    }
    // 004056D8 NiTPointerList AddHead on +0x98 (the host list is head first).
    if world.get::<NiAvProperties>(object).is_none() {
        world.entity_mut(object).insert(NiAvProperties::default());
    }
    world
        .get_mut::<NiAvProperties>(object)
        .expect("just ensured")
        .0
        .insert(0, property);
    Ok(true)
}

// IDA: 007319E0 NiPropertyState_Constructor: ten null slots, then 007317B0 retains the nine process defaults (alpha +0x08,
// material +0x10, renderer-specific +0x14, shade +0x18, stencil +0x1C, texturing +0x20, vertex color +0x24, wireframe
// +0x28, z-buffer +0x2C, in that order); +0x0C fog stays null. Completion: 100% of the store order.
// TODO: [ ] FormHeap ownership.
pub fn construct_scene_property_state(
    world: &mut World,
    defaults: &ScenePropertyDefaults,
) -> Entity {
    world
        .spawn(ScenePropertyState {
            original_vtable: NI_PROPERTY_STATE_VTABLE,
            slots: [
                defaults.alpha,
                None,
                defaults.material,
                defaults.renderer_specific,
                defaults.shade,
                defaults.stencil,
                defaults.texturing,
                defaults.vertex_color,
                defaults.wireframe,
                defaults.zbuffer,
            ],
            host_references: 0,
        })
        .id()
}

// IDA: 007317B0 NiPropertyState_ApplyDefaultProperties(state): the nine globals at +0x08, +0x10..+0x2C replace their slots
// in that order; +0x0C fog keeps whatever 007077D0 merged. A default whose global has cleared nulls its slot.
// Completion: 100%; the refcount swaps are not modelled on properties (`NiProcessDefaults`).
pub fn apply_default_properties(
    world: &mut World,
    state: Entity,
    defaults: &ScenePropertyDefaults,
) {
    let mut owner = world
        .get_mut::<ScenePropertyState>(state)
        .expect("the state is a NiPropertyState");
    owner.slots[0] = defaults.alpha;
    owner.slots[2] = defaults.material;
    owner.slots[3] = defaults.renderer_specific;
    owner.slots[4] = defaults.shade;
    owner.slots[5] = defaults.stencil;
    owner.slots[6] = defaults.texturing;
    owner.slots[7] = defaults.vertex_color;
    owner.slots[8] = defaults.wireframe;
    owner.slots[9] = defaults.zbuffer;
}

// IDA: N/A geometry dropping its +0xAC reference at its own release (NiGeometry's destructor in the original). The state
// goes when this was its last install. Completion: 100%.
pub fn release_geometry_property_state(world: &mut World, geometry: Entity) {
    let mut owner = world
        .get_mut::<NiGeometry>(geometry)
        .expect("a NiGeometry has its links");
    let old = owner.property_state_ac.take();
    replace_reference::<ScenePropertyState>(world, old, None);
}

// IDA: 00731620 NiPropertyState copy constructor: ten null slots, then each slot retains the source slot (equal pointers
// skip). Completion: 100%.
pub fn copy_scene_property_state(world: &mut World, source: Entity) -> Entity {
    let slots = world
        .get::<ScenePropertyState>(source)
        .expect("the source is a NiPropertyState")
        .slots;
    world
        .spawn(ScenePropertyState {
            original_vtable: NI_PROPERTY_STATE_VTABLE,
            slots,
            host_references: 0,
        })
        .id()
}

// IDA: 007077D0 NiAVObject_MergePropertyListIntoState(object, out, inherited, copyState): an empty +0x98 list (+0xA4 count
// 0) returns the inherited state itself; otherwise a copy (copyState, 00731620) or the inherited state itself, then each nonnull list entry from the +0x9C head whose type (vtable +0x4C) is <= 10
// replaces its slot unless already equal. Completion: 100% of the order. A type of 10 is an error (the original writes
// past the 0x30-byte state; no reached property has it). Host substitution, as in the C++ port: a failed FormHeap(0x30) is
// an error that leaves every owner as it was, where the original assigns null and then dereferences it.
pub fn merge_property_list(
    world: &mut World,
    properties: &[Option<Entity>],
    inherited: Option<Entity>,
    copy_state: bool,
) -> Result<Option<Entity>, GameError> {
    // 007077FF: +0xA4 count 0 -> *out = inherited (retained), no lock.
    if properties.is_empty() {
        return Ok(inherited);
    }
    // 00707854: copyState -> FormHeap(0x30) 00731620(inherited); otherwise the inherited state itself. Both dereference
    // a null inherited state.
    let inherited = inherited.ok_or(GameError::unclassified(
        0x007077D0,
        "007077D0: null inherited NiPropertyState",
    ))?;
    let state = if copy_state {
        // The reservation stands for the state's own storage, so this branch stays reachable.
        let mut storage = Vec::<u8>::new();
        storage
            .try_reserve_exact(0x30)
            .map_err(|_| GameError::unclassified(0, "bad allocation"))?;
        let copy = copy_scene_property_state(world, inherited);
        drop(storage);
        copy
    } else {
        inherited
    };
    // 00707898..0070790D: walk from the +0x9C head; null data is skipped.
    for &property in properties.iter().flatten() {
        let Some(kind) = slot_type(world, property)? else {
            continue;
        };
        let mut owner = world
            .get_mut::<ScenePropertyState>(state)
            .expect("the state is a NiPropertyState");
        if owner.slots[kind] != Some(property) {
            owner.slots[kind] = Some(property);
        }
    }
    Ok(Some(state))
}

// IDA: 0070A3E0 NiAVObject_BuildInheritedPropertyState(object): the parent's inherited state (recursive), or a new default
// state (007319E0) at the root, then 007077D0(object list, state, copyState 0) merges the object's own list in place.
// The result is a temporary the caller owns until geometry installs it. Completion: 100% of the recursion and order.
pub fn build_inherited_property_state(
    world: &mut World,
    object: Entity,
    defaults: &ScenePropertyDefaults,
) -> Result<Option<Entity>, GameError> {
    // 0070A425: the parent's state, or a new default state at the root.
    let base = match object_parent(world, object)? {
        Some(parent) => build_inherited_property_state(world, parent, defaults)?,
        None => Some(construct_scene_property_state(world, defaults)),
    };
    // 0070A479 007077D0(object, out, base, 0).
    let list = local_properties(world, object);
    merge_property_list(world, &list, base, false)
}

// IDA: vtable +0x6C UpdatePropertiesDownward(state): NiNode / NiBillboardNode 0070A840 (007077D0 copyState 1, then each
// nonnull child slot's +0x6C); NiTriShape / NiTriStrips, NiScreenElements (vtable A7E29C) and NiParticleSystem 00722B30 (007077D0 copyState 1; the +0xAC
// NiPointer is replaced when it differs); NiDirectionalLight, NiPointLight (A43044) and NiCamera 0068F970 no-op. Completion: 100% for the classes
// bound here.
pub fn update_properties_downward(
    world: &mut World,
    object: Entity,
    state: Option<Entity>,
) -> Result<(), GameError> {
    match world.get::<NiClass>(object).copied() {
        Some(NiClass::Node | NiClass::BillboardNode) => {
            let list = local_properties(world, object);
            let merged = merge_property_list(world, &list, state, true)?;
            let children = world
                .get::<NiNodeChildren>(object)
                .expect("a NiNode has child slots")
                .slots_ac
                .clone();
            for child in children.into_iter().flatten() {
                update_properties_downward(world, child, merged)?;
            }
            if merged != state {
                discard_unreferenced::<ScenePropertyState>(world, merged);
            }
            Ok(())
        }
        Some(NiClass::TriShape | NiClass::TriStrips | NiClass::ScreenElements) => {
            let list = local_properties(world, object);
            let merged = merge_property_list(world, &list, state, true)?;
            let mut geometry = world
                .get_mut::<NiGeometry>(object)
                .expect("a NiGeometry has its links");
            let old = geometry.property_state_ac;
            if old != merged {
                geometry.property_state_ac = merged;
                replace_reference::<ScenePropertyState>(world, old, merged);
            }
            Ok(())
        }
        Some(NiClass::DirectionalLight | NiClass::PointLight | NiClass::Camera) => Ok(()),
        // NiParticleSystem vt+0x6C is the NiGeometry slot.
        Some(NiClass::ParticleSystem) => {
            let list = local_properties(world, object);
            let merged = merge_property_list(world, &list, state, true)?;
            let mut system = world
                .get_mut::<ParticleSystem>(object)
                .expect("a NiParticleSystem has its component");
            let old = system.property_state_ac;
            if old != merged {
                system.property_state_ac = merged;
                replace_reference::<ScenePropertyState>(world, old, merged);
            }
            Ok(())
        }
        _ => Err(GameError::unclassified(
            0,
            "vtable +0x6C on an unbound class",
        )),
    }
}

// IDA: 00707610 NiAVObject_RefreshPropertyState(object): under the 00B3FA00 lock, 0070A3E0 of the +0x1C parent or a new
// default state (007319E0), then the object's vtable +0x6C with it. Completion: 100% of the order.
pub fn refresh_property_state(
    world: &mut World,
    object: Entity,
    defaults: &ScenePropertyDefaults,
) -> Result<(), GameError> {
    let state = match object_parent(world, object)? {
        Some(parent) => build_inherited_property_state(world, parent, defaults)?,
        None => Some(construct_scene_property_state(world, defaults)),
    };
    // 00707702 vtable +0x6C.
    let result = update_properties_downward(world, object, state);
    discard_unreferenced::<ScenePropertyState>(world, state);
    result
}

// IDA: N/A the +0xB0 UID key of a list entry (a light that has gone sorts first and is never produced: the lists are
// rebuilt from live effects).
fn effect_uid(world: &World, entry: Entity) -> u32 {
    world
        .get::<DirectionalLight>(entry)
        .map_or(0, |light| light.uid_b0)
}

// IDA: 00731BF0 sorted insert into a {next, effect} list keyed by the effect +0xB0 UID: a key below the head (or an empty
// list) inserts at the head; otherwise the walk stops at the first entry whose key is >= the new key, and an equal key
// is not inserted again. Completion: 100%.
fn insert_sorted(world: &World, list: &mut Vec<Entity>, effect: Entity) {
    let key = effect_uid(world, effect);
    let position = list
        .iter()
        .position(|&entry| key <= effect_uid(world, entry));
    match position {
        Some(index) if effect_uid(world, list[index]) == key => {}
        Some(index) => list.insert(index, effect),
        None => list.push(effect),
    }
}

// IDA: 00731C70 removes the first entry with an equal UID (the walk stops at the first key >= the effect's).
// Completion: 100%.
fn remove_sorted(world: &World, list: &mut Vec<Entity>, effect: Entity) {
    let key = effect_uid(world, effect);
    let position = list
        .iter()
        .position(|&entry| key <= effect_uid(world, entry));
    if let Some(index) = position
        && effect_uid(world, list[index]) == key
    {
        list.remove(index);
    }
}

// IDA: N/A runs a list edit on +0x0C. The list leaves the component while the edit reads the lights' UIDs.
fn edit_lights(world: &mut World, state: Entity, edit: impl FnOnce(&World, &mut Vec<Entity>)) {
    let mut list = std::mem::take(
        &mut world
            .get_mut::<SceneEffectState>(state)
            .expect("the state is a NiDynamicEffectState")
            .lights_0c,
    );
    edit(world, &mut list);
    world
        .get_mut::<SceneEffectState>(state)
        .expect("the state is a NiDynamicEffectState")
        .lights_0c = list;
}

// IDA: 00731CE0 NiDynamicEffectState add: vtable +0x84 GetEffectType of NiDirectionalLight (0091FEA0) returns 1, a light
// type (<= 3), so it goes to the +0x0C list (00731BF0) without touching +0x08. Completion: 100% for NiDirectionalLight.
fn add_effect(world: &mut World, state: Entity, effect: Entity) {
    edit_lights(world, state, |world, list| {
        insert_sorted(world, list, effect);
    });
}

// IDA: 00731D80 NiDynamicEffectState remove: a type <= 3 leaves the +0x0C list (00731C70); +0x08 is recomputed only for
// the other types. Completion: 100%.
fn remove_effect(world: &mut World, state: Entity, effect: Entity) {
    edit_lights(world, state, |world, list| {
        remove_sorted(world, list, effect);
    });
}

// IDA: 00731B60 NiDynamicEffectState copy: a new 0x20 state, +0x08 copied, the three lists copied node by node (00731A90),
// +0x18 / +0x1C copied. Completion: 100%.
fn copy_effect_state(world: &mut World, source: Entity) -> Entity {
    let mut copy = world
        .get::<SceneEffectState>(source)
        .expect("the source is a NiDynamicEffectState")
        .clone();
    copy.host_references = 0;
    world.spawn(copy).id()
}

// IDA: 00708CE0: true when the node is in the effect's +0xCC list.
fn effect_excludes(world: &World, effect: Entity, node: Entity) -> bool {
    world
        .get::<DirectionalLight>(effect)
        .is_some_and(|light| light.stream_node_list_cc.contains(&node))
}

// IDA: 0070A500 NiNode_MergeEffectListIntoState(node, out, inherited, copy): effects of the inherited lists whose +0xCC
// list holds this node (00708CE0) are removed from a copy made once (00731B60); then a node with an empty +0xBC effect
// list (+0xC8 count 0) returns that state; otherwise a copy (copy flag), the inherited state itself, or a new 00709E60
// state at the root receives each +0xBC effect head first (00731CE0). Completion: 100% of the order.
pub fn merge_effect_list(
    world: &mut World,
    node: Entity,
    inherited: Option<Entity>,
    copy_state: bool,
) -> Option<Entity> {
    let mut state = inherited;
    let mut copied = false;
    if let Some(source) = inherited {
        // 0070A557..0070A750: the inherited lists (+0x0C, +0x10, +0x14) and maps (+0x18, +0x1C) are read from the original
        // state; an effect excluding this node is removed from a copy made once.
        let original = world
            .get::<SceneEffectState>(source)
            .expect("the inherited state is a NiDynamicEffectState")
            .clone();
        let listed = original
            .lights_0c
            .iter()
            .chain(&original.projected_lights_10)
            .chain(&original.projected_shadows_14)
            .chain(&original.fog_map_18)
            .chain(&original.environment_map_1c);
        for &effect in listed {
            if !effect_excludes(world, effect, node) {
                continue;
            }
            if !copied {
                state = Some(copy_effect_state(world, source));
                copied = true;
            }
            remove_effect(world, state.expect("just copied"), effect);
        }
    }
    // 0070A75B: an empty +0xBC effect list (+0xC8 count 0) returns here.
    let effects = world
        .get::<NiNodeEffects>(node)
        .expect("a NiNode has an effect list")
        .0
        .clone();
    if effects.is_empty() {
        return state;
    }
    match inherited {
        // 0070A7A3: copy flag -> a copy (unless already copied); otherwise the inherited state itself (unless already
        // copied).
        Some(source) => {
            if !copied && copy_state {
                state = Some(copy_effect_state(world, source));
            }
        }
        // 0070A783 FormHeap(0x20) 00709E60.
        None => state = Some(world.spawn(SceneEffectState::default()).id()),
    }
    // 0070A7C5..0070A7DA: each +0xBC effect from the +0xC0 head (00731CE0).
    let target = state.expect("a state exists from here on");
    for effect in effects {
        if world.get::<DirectionalLight>(effect).is_some() {
            add_effect(world, target, effect);
        }
    }
    state
}

// IDA: 0070B400 NiAVObject_BuildInheritedEffectState(object): the parent's state (recursive; null at the root), then
// 0070A500(object, state, 0). Only NiNode-family objects are ever passed (00707980 starts at the parent). The result is
// a temporary the caller owns until geometry installs it. Completion: 100% of the recursion and order.
pub fn build_inherited_effect_state(
    world: &mut World,
    node: Entity,
) -> Result<Option<Entity>, GameError> {
    let base = match object_parent(world, node)? {
        Some(parent) => build_inherited_effect_state(world, parent)?,
        None => None,
    };
    // 0070B495 0070A500(this, out, base, 0). An exclusion answers with a copy, which drops the last hold on the base.
    let merged = merge_effect_list(world, node, base, false);
    if merged != base {
        discard_unreferenced::<SceneEffectState>(world, base);
    }
    Ok(merged)
}

// IDA: vtable +0x70 UpdateEffectsDownward(state): NiNode / NiBillboardNode 0070A900 (0070A500 copy 1, then each nonnull
// child slot's +0x70); NiTriShape / NiTriStrips and NiScreenElements 00722C00 (+0xB0 = state, or released for null); NiDirectionalLight and
// NiCamera 0068F970 no-op; NiParticleSystem 00742060. Completion: 100% for the bound classes; 00742060's lights-only copy stops.
pub fn update_effects_downward(
    world: &mut World,
    object: Entity,
    state: Option<Entity>,
) -> Result<(), GameError> {
    match world.get::<NiClass>(object).copied() {
        Some(NiClass::Node | NiClass::BillboardNode) => {
            let merged = merge_effect_list(world, object, state, true);
            let children = world
                .get::<NiNodeChildren>(object)
                .expect("a NiNode has child slots")
                .slots_ac
                .clone();
            for child in children.into_iter().flatten() {
                update_effects_downward(world, child, merged)?;
            }
            if merged != state {
                discard_unreferenced::<SceneEffectState>(world, merged);
            }
            Ok(())
        }
        Some(NiClass::TriShape | NiClass::TriStrips | NiClass::ScreenElements) => {
            let mut geometry = world
                .get_mut::<NiGeometry>(object)
                .expect("a NiGeometry has its links");
            let old = geometry.effect_state_b0;
            if old != state {
                geometry.effect_state_b0 = state;
                replace_reference::<SceneEffectState>(world, old, state);
            }
            Ok(())
        }
        Some(NiClass::DirectionalLight | NiClass::PointLight | NiClass::Camera) => Ok(()),
        // 00742060: a state holding projected lights, shadows or a fog or environment map would be copied as a new state of
        // its +0x0C lights only (a stop); otherwise it is installed at +0xB0 as geometry's is, and null releases +0xB0.
        Some(NiClass::ParticleSystem) => {
            if let Some(held) = state.and_then(|state| world.get::<SceneEffectState>(state))
                && (!held.projected_lights_10.is_empty()
                    || !held.projected_shadows_14.is_empty()
                    || held.fog_map_18.is_some()
                    || held.environment_map_1c.is_some())
            {
                return Err(GameError::unclassified(
                    0x00742083,
                    "00742083: a particle system's lights-only effect state",
                ));
            }
            let mut system = world
                .get_mut::<super::particle_system::ParticleSystem>(object)
                .expect("a NiParticleSystem has its component");
            let old = system.effect_state_b0;
            if old != state {
                system.effect_state_b0 = state;
                replace_reference::<SceneEffectState>(world, old, state);
            }
            Ok(())
        }
        _ => Err(GameError::unclassified(
            0,
            "vtable +0x70 on an unbound class",
        )),
    }
}

// IDA: 00707980 NiAVObject_RefreshEffectState(object): 0070B400 of the +0x1C parent (null without one), then the object's
// vtable +0x70. Completion: 100%.
pub fn refresh_effect_state(world: &mut World, object: Entity) -> Result<(), GameError> {
    let state = match object_parent(world, object)? {
        Some(parent) => build_inherited_effect_state(world, parent)?,
        None => None,
    };
    update_effects_downward(world, object, state)?;
    discard_unreferenced::<SceneEffectState>(world, state);
    Ok(())
}

// IDA: 007319E0's defaults from the world.
pub(crate) fn property_defaults(world: &World) -> ScenePropertyDefaults {
    world
        .resource::<NiProcessDefaults>()
        .scene_property_defaults()
}

#[cfg(test)]
mod tests {
    use super::super::mesh_geometry::{MeshTriShapePayload, spawn_mesh_geometry};
    use super::super::object_net::NiObjectNet;
    use super::super::particle_system::spawn_particle_system;
    use super::super::scene_derived::spawn_directional_light;
    use super::super::scene_node::{SceneNodePayload, spawn_scene_node};
    use super::*;

    fn property(world: &mut World, class: NiClass) -> Entity {
        world.spawn((class, NiObjectNet::default())).id()
    }

    fn node(world: &mut World) -> Entity {
        spawn_scene_node(world, SceneNodePayload::default())
    }

    fn shape(world: &mut World) -> Entity {
        spawn_mesh_geometry(world, NiClass::TriShape, MeshTriShapePayload::default())
    }

    fn attach(world: &mut World, parent: Entity, child: Entity) {
        world
            .get_mut::<NiNodeChildren>(parent)
            .unwrap()
            .slots_ac
            .push(Some(child));
        world.entity_mut(child).insert(NiAvParent(Some(parent)));
    }

    fn state_count<T: Component>(world: &mut World) -> usize {
        world.query::<&T>().iter(world).count()
    }

    #[test]
    fn property_types_and_attach() {
        let mut world = World::new();
        for (class, expected) in [
            (NiClass::AlphaProperty, 0),
            (NiClass::MaterialProperty, 2),
            (NiClass::StencilProperty, 5),
            (NiClass::TexturingProperty, 6),
            (NiClass::VertexColorProperty, 7),
            (NiClass::ZBufferProperty, 9),
        ] {
            let entity = property(&mut world, class);
            assert_eq!(property_type(&world, Some(entity)).unwrap(), expected);
        }
        let fog = world.spawn(FogProperty::default()).id();
        assert_eq!(property_type(&world, Some(fog)).unwrap(), 1);
        let sky = world.spawn(SkyShaderProperty::default()).id();
        assert_eq!(property_type(&world, Some(sky)).unwrap(), 4);
        let wireframe = world.spawn(WireframeProperty::default()).id();
        assert_eq!(property_type(&world, Some(wireframe)).unwrap(), 8);
        for (entity, expected) in [
            (
                world
                    .spawn(NiFogProperty {
                        flags_18: 0,
                        depth_1c: 1.0,
                        color_20: [0.0; 3],
                    })
                    .id(),
                1,
            ),
            (world.spawn(NiRendererSpecificProperty).id(), 3),
            (world.spawn(NiShadeProperty { flags_18: 1 }).id(), 4),
        ] {
            assert_eq!(property_type(&world, Some(entity)).unwrap(), expected);
        }
        assert!(property_type(&world, None).is_err());
        let data = world.spawn(NiClass::FloatKeyData).id();
        assert!(property_type(&world, Some(data)).is_err());

        let object = shape(&mut world);
        assert!(attach_object_property(&mut world, object, Some(fog)).unwrap());
        assert!(attach_object_property(&mut world, object, Some(sky)).unwrap());
        assert_eq!(
            world.get::<NiAvProperties>(object).unwrap().0,
            [Some(sky), Some(fog)],
            "AddHead"
        );
        assert!(attach_object_property(&mut world, object, None).is_err());
        assert!(attach_object_property(&mut world, data, Some(fog)).is_err());
    }

    #[test]
    fn default_state_skips_fog_and_copies_slots() {
        let mut world = World::new();
        let alpha = property(&mut world, NiClass::AlphaProperty);
        let zbuffer = property(&mut world, NiClass::ZBufferProperty);
        let defaults = ScenePropertyDefaults {
            alpha: Some(alpha),
            zbuffer: Some(zbuffer),
            ..ScenePropertyDefaults::default()
        };
        let state = construct_scene_property_state(&mut world, &defaults);
        let owner = world.get::<ScenePropertyState>(state).unwrap();
        assert_eq!(owner.original_vtable, 0x00A8_0984);
        assert_eq!(owner.slots[0], Some(alpha));
        assert_eq!(owner.slots[1], None);
        assert_eq!(owner.slots[9], Some(zbuffer));
        let copy = copy_scene_property_state(&mut world, state);
        assert_ne!(copy, state);
        assert_eq!(
            world.get::<ScenePropertyState>(copy).unwrap().slots,
            world.get::<ScenePropertyState>(state).unwrap().slots
        );
    }

    #[test]
    fn merge_returns_the_inherited_state_or_a_copy() {
        let mut world = World::new();
        let state = construct_scene_property_state(&mut world, &ScenePropertyDefaults::default());
        let first = property(&mut world, NiClass::AlphaProperty);
        let last = property(&mut world, NiClass::AlphaProperty);

        assert_eq!(
            merge_property_list(&mut world, &[], Some(state), true).unwrap(),
            Some(state),
            "an empty list returns the inherited state itself, even with the copy flag"
        );
        assert_eq!(
            merge_property_list(&mut world, &[], None, true).unwrap(),
            None
        );
        assert!(merge_property_list(&mut world, &[Some(first)], None, true).is_err());

        let copy = merge_property_list(
            &mut world,
            &[Some(first), None, Some(last)],
            Some(state),
            true,
        )
        .unwrap()
        .unwrap();
        assert_ne!(copy, state);
        assert_eq!(
            world.get::<ScenePropertyState>(copy).unwrap().slots[0],
            Some(last),
            "a later list entry of one type wins"
        );
        assert_eq!(
            world.get::<ScenePropertyState>(state).unwrap().slots[0],
            None,
            "the copy leaves the inherited state alone"
        );

        let same = merge_property_list(&mut world, &[Some(first)], Some(state), false).unwrap();
        assert_eq!(same, Some(state), "copyState 0 merges in place");
        assert_eq!(
            world.get::<ScenePropertyState>(state).unwrap().slots[0],
            Some(first)
        );
    }

    /// 00707610 over a tree: the chain's lists merge, geometry installs, and an empty geometry list shares the state.
    #[test]
    fn refresh_pushes_states_down_to_geometry() {
        let mut world = World::new();
        let default_alpha = property(&mut world, NiClass::AlphaProperty);
        let defaults = ScenePropertyDefaults {
            alpha: Some(default_alpha),
            ..ScenePropertyDefaults::default()
        };
        let root = node(&mut world);
        let middle = node(&mut world);
        let plain = shape(&mut world);
        let own = shape(&mut world);
        let light = spawn_directional_light(&mut world);
        attach(&mut world, root, middle);
        attach(&mut world, middle, plain);
        attach(&mut world, middle, own);
        attach(&mut world, middle, light);
        let root_alpha = property(&mut world, NiClass::AlphaProperty);
        let own_stencil = property(&mut world, NiClass::StencilProperty);
        attach_object_property(&mut world, root, Some(root_alpha)).unwrap();
        attach_object_property(&mut world, own, Some(own_stencil)).unwrap();

        refresh_property_state(&mut world, root, &defaults).unwrap();
        let plain_state = world
            .get::<NiGeometry>(plain)
            .unwrap()
            .property_state_ac
            .unwrap();
        let own_state = world
            .get::<NiGeometry>(own)
            .unwrap()
            .property_state_ac
            .unwrap();
        assert_ne!(plain_state, own_state);
        let plain_owner = world.get::<ScenePropertyState>(plain_state).unwrap();
        assert_eq!(plain_owner.slots[0], Some(root_alpha));
        assert_eq!(plain_owner.host_references, 1);
        let own_owner = world.get::<ScenePropertyState>(own_state).unwrap();
        assert_eq!(own_owner.slots[0], Some(root_alpha));
        assert_eq!(own_owner.slots[5], Some(own_stencil));
        assert_eq!(
            state_count::<ScenePropertyState>(&mut world),
            2,
            "the root's default and merged temporaries are gone"
        );

        // A refresh from the leaf builds the parent chain, and the old state is released.
        refresh_property_state(&mut world, plain, &defaults).unwrap();
        let replaced = world
            .get::<NiGeometry>(plain)
            .unwrap()
            .property_state_ac
            .unwrap();
        assert_eq!(
            world.get::<ScenePropertyState>(replaced).unwrap().slots[0],
            Some(root_alpha)
        );
        assert!(world.get::<ScenePropertyState>(plain_state).is_none());
        assert_eq!(state_count::<ScenePropertyState>(&mut world), 2);

        // 00722B30 (NiParticleSystem vt+0x6C is the NiGeometry slot): the +0xAC state takes the system's own properties.
        let particles = spawn_particle_system(&mut world);
        let own_alpha = world
            .spawn((NiClass::AlphaProperty, AlphaProperty::default()))
            .id();
        world.get_mut::<NiAvProperties>(particles).unwrap().0 = vec![Some(own_alpha)];
        refresh_property_state(&mut world, particles, &defaults).unwrap();
        let state = world
            .get::<ParticleSystem>(particles)
            .unwrap()
            .property_state_ac
            .unwrap();
        let owner = world.get::<ScenePropertyState>(state).unwrap();
        assert_eq!(owner.slots[0], Some(own_alpha));
        assert_eq!(owner.host_references, 1);
    }

    /// 00731BF0's list is sorted by UID whatever the attach order, and equal keys are not inserted twice.
    #[test]
    fn effect_lists_sort_by_uid() {
        let mut world = World::new();
        let first = spawn_directional_light(&mut world);
        let second = spawn_directional_light(&mut world);
        let third = spawn_directional_light(&mut world);
        let parent = node(&mut world);
        let state = merge_effect_list(&mut world, parent, None, false);
        assert_eq!(state, None, "no inherited state and no effects");
        let target = world.spawn(SceneEffectState::default()).id();
        for effect in [third, first, second, first] {
            add_effect(&mut world, target, effect);
        }
        assert_eq!(
            world.get::<SceneEffectState>(target).unwrap().lights_0c,
            [first, second, third]
        );
        remove_effect(&mut world, target, second);
        assert_eq!(
            world.get::<SceneEffectState>(target).unwrap().lights_0c,
            [first, third]
        );
        remove_effect(&mut world, target, second);
        assert_eq!(
            world.get::<SceneEffectState>(target).unwrap().lights_0c,
            [first, third],
            "an absent key removes nothing"
        );
    }

    /// 0070A500's branches: exclusion copies once, an empty list returns, the copy flag decides the rest.
    #[test]
    fn effect_merge_branches() {
        let mut world = World::new();
        let excluded = spawn_directional_light(&mut world);
        let kept = spawn_directional_light(&mut world);
        let local = spawn_directional_light(&mut world);
        let plain = node(&mut world);
        let excluding = node(&mut world);
        world
            .get_mut::<DirectionalLight>(excluded)
            .unwrap()
            .stream_node_list_cc = vec![excluding];
        let inherited = world
            .spawn(SceneEffectState {
                lights_0c: vec![excluded, kept],
                ..SceneEffectState::default()
            })
            .id();

        assert_eq!(
            merge_effect_list(&mut world, plain, Some(inherited), true),
            Some(inherited),
            "no exclusion and no effects: the inherited state itself, even with the copy flag"
        );
        let copied = merge_effect_list(&mut world, excluding, Some(inherited), false).unwrap();
        assert_ne!(copied, inherited, "exclusion copies even without the flag");
        assert_eq!(
            world.get::<SceneEffectState>(copied).unwrap().lights_0c,
            [kept]
        );
        assert_eq!(
            world.get::<SceneEffectState>(inherited).unwrap().lights_0c,
            [excluded, kept]
        );

        world.get_mut::<NiNodeEffects>(plain).unwrap().0 = vec![local];
        let copy = merge_effect_list(&mut world, plain, Some(inherited), true).unwrap();
        assert_ne!(copy, inherited);
        assert_eq!(
            world.get::<SceneEffectState>(copy).unwrap().lights_0c,
            [excluded, kept, local]
        );
        assert_eq!(
            merge_effect_list(&mut world, plain, Some(inherited), false),
            Some(inherited),
            "without the flag the effect goes into the inherited state"
        );
        assert_eq!(
            world.get::<SceneEffectState>(inherited).unwrap().lights_0c,
            [excluded, kept, local]
        );
        let fresh = merge_effect_list(&mut world, plain, None, true).unwrap();
        assert_eq!(
            world
                .get::<SceneEffectState>(fresh)
                .unwrap()
                .original_vtable,
            0x00A7_E350
        );
        assert_eq!(
            world.get::<SceneEffectState>(fresh).unwrap().lights_0c,
            [local]
        );
    }

    /// 00707980 over a tree: a null state reaches geometry; a parent's effect reaches it; a shared state stays shared.
    #[test]
    fn refresh_effect_state_installs_on_geometry() {
        let mut world = World::new();
        let root = node(&mut world);
        let first = shape(&mut world);
        let second = shape(&mut world);
        attach(&mut world, root, first);
        attach(&mut world, root, second);
        refresh_effect_state(&mut world, root).unwrap();
        assert_eq!(
            world.get::<NiGeometry>(first).unwrap().effect_state_b0,
            None
        );

        let light = spawn_directional_light(&mut world);
        world.get_mut::<NiNodeEffects>(root).unwrap().0 = vec![light];
        refresh_effect_state(&mut world, root).unwrap();
        let state = world
            .get::<NiGeometry>(first)
            .unwrap()
            .effect_state_b0
            .unwrap();
        assert_eq!(
            world.get::<NiGeometry>(second).unwrap().effect_state_b0,
            Some(state),
            "both children hold the node's one merged state"
        );
        let owner = world.get::<SceneEffectState>(state).unwrap();
        assert_eq!(owner.lights_0c, [light]);
        assert_eq!(owner.host_references, 2);

        // A refresh from a child starts at the parent, whose merge has no copy flag.
        refresh_effect_state(&mut world, first).unwrap();
        let rebuilt = world
            .get::<NiGeometry>(first)
            .unwrap()
            .effect_state_b0
            .unwrap();
        assert_ne!(rebuilt, state);
        assert_eq!(
            world
                .get::<SceneEffectState>(state)
                .unwrap()
                .host_references,
            1
        );

        // The null state releases what geometry held.
        world.get_mut::<NiNodeEffects>(root).unwrap().0.clear();
        refresh_effect_state(&mut world, root).unwrap();
        assert_eq!(
            world.get::<NiGeometry>(first).unwrap().effect_state_b0,
            None
        );
        assert_eq!(state_count::<SceneEffectState>(&mut world), 0);

        // 00742060 installs a light-only state at the particle system's +0xB0, as geometry does.
        let particles = spawn_particle_system(&mut world);
        attach(&mut world, root, particles);
        world.get_mut::<NiNodeEffects>(root).unwrap().0.push(light);
        refresh_effect_state(&mut world, root).unwrap();
        let held = world
            .get::<super::super::particle_system::ParticleSystem>(particles)
            .unwrap()
            .effect_state_b0
            .unwrap();
        assert_eq!(
            world.get::<SceneEffectState>(held).unwrap().lights_0c,
            [light]
        );
    }

    /// An exclusion below the root copies the temporary, and the temporary it replaced does not stay behind.
    #[test]
    fn build_inherited_effect_state_drops_the_replaced_temporary() {
        let mut world = World::new();
        let root = node(&mut world);
        let middle = node(&mut world);
        attach(&mut world, root, middle);
        let light = spawn_directional_light(&mut world);
        world.get_mut::<NiNodeEffects>(root).unwrap().0 = vec![light];
        world
            .get_mut::<DirectionalLight>(light)
            .unwrap()
            .stream_node_list_cc = vec![middle];
        let state = build_inherited_effect_state(&mut world, middle)
            .unwrap()
            .unwrap();
        assert!(
            world
                .get::<SceneEffectState>(state)
                .unwrap()
                .lights_0c
                .is_empty()
        );
        assert_eq!(state_count::<SceneEffectState>(&mut world), 1);
    }

    #[test]
    fn expired_parent_is_a_host_error() {
        let mut world = World::new();
        let child = shape(&mut world);
        let parent = node(&mut world);
        world.entity_mut(child).insert(NiAvParent(Some(parent)));
        world.despawn(parent);
        assert_eq!(
            refresh_effect_state(&mut world, child).unwrap_err(),
            GameError::unclassified(0, "NiAVObject +0x1C parent owner expired")
        );
    }
}
