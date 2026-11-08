//! What a NiStream's objects are: entities with a class tag, listed in stream order.

use bevy_ecs::prelude::*;

/// A null stream reference.
pub const NULL_REFERENCE: u32 = 0xFFFF_FFFF;

/// IDA: the most-derived class of a stream object, in place of a `FactoryObject` variant. The original
/// reinterprets pointers without RTTI at these sites, and the C++ port checks typed identity. So does
/// this tag. Variants are added as their classes are ported.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NiClass {
    StringExtraData,
    BinaryExtraData,
    TextKeyExtraData,
    BsxFlagsExtraData,
    BsFurnitureMarker,
    BsBound,
    BoneLodController,
    BhkBlendController,
    /// 004D67C0 TESObjectExtraData ("REF"), which Load3D adds (`tes::reference_3d`).
    TesObjectExtraData,
    AlphaProperty,
    ZBufferProperty,
    VertexColorProperty,
    StencilProperty,
    SpecularProperty,
    MaterialProperty,
    TexturingProperty,
    Node,
    BillboardNode,
    TriShape,
    TriStrips,
    ParticleSystem,
    ParticleData,
    ParticlePositionModifier,
    ParticleGrowFadeModifier,
    ParticleAgeDeathModifier,
    ParticleBoundUpdateModifier,
    ParticleColorModifier,
    ParticleDragModifier,
    ParticleGravityModifier,
    ParticleRotationModifier,
    ParticleSpawnModifier,
    ParticleBoxEmitter,
    ParticleSphereEmitter,
    ParticleCylinderEmitter,
    ParticleMeshEmitter,
    BsParentVelocityModifier,
    DirectionalLight,
    /// 004B1600's NiPointLight; no stream factory reaches it yet.
    PointLight,
    Camera,
    /// The load-only `bhk*` classes (`bs::havok_stream`).
    BhkSpCollisionObject,
    BhkSimpleShapePhantom,
    BhkConvexVerticesShape,
    /// The prison's `bhk*` classes, loaded and linked as data (`bs::havok_stream`).
    BhkCollisionObject,
    BhkRigidBody,
    BhkRigidBodyT,
    BhkNiTriStripsShape,
    BhkMoppBvTreeShape,
    BhkBoxShape,
    BhkCapsuleShape,
    BhkConvexTransformShape,
    BhkListShape,
    BhkTransformShape,
    BhkRagdollConstraint,
    BhkMalleableConstraint,
    BhkBlendCollisionObject,
    BhkBlendCollisionAddRotation,
    BhkSphereShape,
    BhkLimitedHingeConstraint,
    BhkPrismaticConstraint,
    ParticleUpdateController,
    ParticleEmitterController,
    ParticleResetOnLoopController,
    ParticleModifierActiveController,
    ParticleGravityStrengthController,
    ParticleEmitterLifeSpanController,
    TransformController,
    VisController,
    MultiTargetTransformController,
    ControllerManager,
    FloatKeyData,
    BoolKeyData,
    ColorKeyData,
    PositionKeyData,
    TransformKeyData,
    FloatInterpolator,
    BoolInterpolator,
    BoolTimelineInterpolator,
    TransformInterpolator,
    PathInterpolator,
    BlendBoolInterpolator,
    BlendFloatInterpolator,
    BlendTransformInterpolator,
    BSplineData,
    BSplineBasisData,
    BSplineCompTransformInterpolator,
    BSplineCompFloatInterpolator,
    /// 006C37D0, made only by 006C3B70 for a cumulative manager's accum root (never streamed).
    BlendAccumTransformInterpolator,
    StringPalette,
    SourceTexture,
    PixelData,
    TriShapeData,
    TriStripsData,
    SkinData,
    SkinInstance,
    SkinPartition,
    DefaultAvObjectPalette,
    ScreenElements,
    ControllerSequence,
    FlipController,
    TextureTransformController,
    AlphaController,
    MaterialColorController,
    Point3Interpolator,
    BlendPoint3Interpolator,
    GeomMorpherController,
    MorphData,
}

impl NiClass {
    /// IDA: the NiExtraData classes bound so far; 00700050's loop accepts these.
    #[must_use]
    pub const fn is_extra_data(self) -> bool {
        matches!(
            self,
            Self::StringExtraData
                | Self::BinaryExtraData
                | Self::TextKeyExtraData
                | Self::BsxFlagsExtraData
                | Self::BsFurnitureMarker
                | Self::BsBound
                | Self::TesObjectExtraData
        )
    }

    /// IDA: the six NiProperty factories reached at startup (007081B0's list).
    #[must_use]
    pub const fn is_property(self) -> bool {
        matches!(
            self,
            Self::AlphaProperty
                | Self::SpecularProperty
                | Self::ZBufferProperty
                | Self::VertexColorProperty
                | Self::StencilProperty
                | Self::MaterialProperty
                | Self::TexturingProperty
        )
    }

    /// IDA: the NiAVObject classes 0070B250 receives.
    #[must_use]
    pub const fn is_av_object(self) -> bool {
        matches!(
            self,
            Self::Node
                | Self::BillboardNode
                | Self::TriShape
                | Self::TriStrips
                | Self::ScreenElements
                | Self::ParticleSystem
                | Self::DirectionalLight
                | Self::PointLight
                | Self::Camera
        )
    }

    /// IDA: the NiNode family, which has child slots.
    #[must_use]
    pub const fn is_node(self) -> bool {
        matches!(self, Self::Node | Self::BillboardNode)
    }

    /// IDA: the particle controller heads 00700050 accepts on a particle-side owner.
    #[must_use]
    pub const fn is_particle_controller(self) -> bool {
        matches!(
            self,
            Self::ParticleUpdateController
                | Self::ParticleEmitterController
                | Self::ParticleResetOnLoopController
                | Self::ParticleModifierActiveController
                | Self::ParticleGravityStrengthController
                | Self::ParticleEmitterLifeSpanController
        )
    }

    /// IDA: 00749BC0's NiPSysModifier classes: the fourteen a NiParticleSystem attaches through 00749990.
    #[must_use]
    pub const fn is_particle_modifier(self) -> bool {
        matches!(
            self,
            Self::ParticlePositionModifier
                | Self::ParticleGrowFadeModifier
                | Self::ParticleAgeDeathModifier
                | Self::ParticleBoundUpdateModifier
                | Self::ParticleColorModifier
                | Self::ParticleDragModifier
                | Self::ParticleGravityModifier
                | Self::ParticleRotationModifier
                | Self::ParticleSpawnModifier
                | Self::ParticleBoxEmitter
                | Self::ParticleSphereEmitter
                | Self::ParticleCylinderEmitter
                | Self::ParticleMeshEmitter
                | Self::BsParentVelocityModifier
        )
    }

    /// IDA: the node controller heads and +0x34 next links the C++ port binds on a NiNode.
    #[must_use]
    pub const fn is_node_controller(self) -> bool {
        matches!(
            self,
            Self::TransformController
                | Self::VisController
                | Self::MultiTargetTransformController
                | Self::ControllerManager
                | Self::BoneLodController
                | Self::BhkBlendController
        )
    }

    /// IDA: the controllers whose target is a NiTexturingProperty.
    #[must_use]
    pub const fn is_texture_controller(self) -> bool {
        matches!(
            self,
            Self::FlipController | Self::TextureTransformController
        )
    }

    /// IDA: every controller class 00700050 can publish at +0x0C in the host. The original has no type check.
    #[must_use]
    pub const fn is_controller_head(self) -> bool {
        self.is_particle_controller()
            || self.is_node_controller()
            || self.is_texture_controller()
            || matches!(
                self,
                Self::AlphaController | Self::MaterialColorController | Self::GeomMorpherController
            )
    }

    /// IDA: 006CBCD0 accepts items whose vtable +0x78 is true.
    #[must_use]
    pub const fn is_blend_transform_source(self) -> bool {
        matches!(
            self,
            Self::TransformInterpolator
                | Self::BlendTransformInterpolator
                | Self::PathInterpolator
                | Self::BSplineCompTransformInterpolator
        )
    }

    /// IDA: the sources 006EB570 / 006EB5E0 call through +0x60.
    #[must_use]
    pub const fn is_blend_bool_source(self) -> bool {
        matches!(
            self,
            Self::BoolInterpolator | Self::BoolTimelineInterpolator | Self::BlendBoolInterpolator
        )
    }

    /// IDA: the sources 006D2580 / 006D2600 call through +0x5C.
    #[must_use]
    pub const fn is_blend_float_source(self) -> bool {
        matches!(
            self,
            Self::FloatInterpolator
                | Self::BlendFloatInterpolator
                | Self::BSplineCompFloatInterpolator
        )
    }

    /// IDA: the Point3 sources a NiBlendPoint3Interpolator (006EA8C0) takes.
    #[must_use]
    pub const fn is_blend_point3_source(self) -> bool {
        matches!(
            self,
            Self::Point3Interpolator | Self::BlendPoint3Interpolator
        )
    }

    /// IDA: 006C8820's InterpArrayItem +0 interpolator classes.
    #[must_use]
    pub const fn is_sequence_interpolator(self) -> bool {
        self.is_blend_transform_source()
            || self.is_blend_bool_source()
            || self.is_blend_float_source()
            || self.is_blend_point3_source()
    }

    /// IDA: 006C8820's InterpArrayItem +4 classes, whose virtuals +58 / +7C / +80 / +84 / +8C / +98 are ported.
    #[must_use]
    pub const fn is_sequence_controller(self) -> bool {
        matches!(
            self,
            Self::TransformController
                | Self::VisController
                | Self::MaterialColorController
                | Self::MultiTargetTransformController
                | Self::ParticleEmitterController
                | Self::ParticleModifierActiveController
                | Self::ParticleGravityStrengthController
        )
    }

    /// IDA: the RTTI prefix before "ExtraData" that 006FF8A0 copies. "BSXFlags" has no "ExtraData"
    /// substring, so its prefix is empty.
    #[must_use]
    pub const fn extra_rtti_prefix(self) -> &'static str {
        match self {
            Self::StringExtraData => "NiString",
            Self::BinaryExtraData => "NiBinary",
            Self::TextKeyExtraData => "NiTextKey",
            _ => "",
        }
    }
}

/// IDA: 00713FF0's type row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamTypeEntry {
    pub name: String,
    pub original_constructor: u32,
}

/// IDA: 00713FF0's object row. `entity` holds the object, whatever its class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamObjectEntry {
    pub type_index: u16,
    pub requested_name: String,
    pub original_constructor: u32,
    pub entity: Entity,
}

/// IDA: the objects of one NiStream, in stream order. References are indexes into it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamObjectTable {
    pub types: Vec<StreamTypeEntry>,
    pub objects: Vec<StreamObjectEntry>,
    pub bytes_consumed: usize,
}

impl StreamObjectTable {
    /// IDA: N/A adds an existing entity at the next index.
    pub fn push(&mut self, name: &str, original_constructor: u32, entity: Entity) {
        self.objects.push(StreamObjectEntry {
            type_index: 0,
            requested_name: name.to_owned(),
            original_constructor,
            entity,
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamObjectErrorCode {
    UnsupportedVersion,
    TruncatedTable,
    NameTooLong,
    MissingRegistration,
    InvalidTypeIndex,
    UnsupportedFactory,
}

/// IDA: N/A a structured failure of 00713FF0's bounded table. Only a missing registration maps to an
/// original error code, 5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamObjectError {
    pub code: StreamObjectErrorCode,
    pub byte_offset: usize,
    pub type_index: Option<usize>,
    pub object_index: Option<usize>,
    pub name: String,
}

impl StreamObjectError {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn original_error_code(&self) -> Option<u32> {
        match self.code {
            StreamObjectErrorCode::MissingRegistration => Some(5),
            _ => None,
        }
    }
}

// IDA: N/A a helper, with no address of its own.
fn table_error(
    code: StreamObjectErrorCode,
    byte_offset: usize,
    type_index: Option<usize>,
    object_index: Option<usize>,
    name: String,
) -> StreamObjectError {
    StreamObjectError {
        code,
        byte_offset,
        type_index,
        object_index,
        name,
    }
}

/// IDA: N/A the objects a failed table call made so far.
fn despawn_created(world: &mut World, table: &StreamObjectTable) {
    for entry in &table.objects {
        world.despawn(entry.entity);
    }
}

/// IDA: 00713FF0 NiStream_ReadTypeTableAndCreateObjects, gated by 00714390 at 05000001: the type names, each
/// looked up even when no object uses it; then every object's index, read before any constructor runs, so a
/// bad index creates nothing; then one factory call per object, in order. A failed factory call midway despawns
/// the objects made so far, as dropping the C++ table does, so a failure publishes nothing. Completion: 100% of
/// the table.
pub fn read_stream_object_table(
    world: &mut World,
    bytes: &[u8],
    object_count: u32,
    version: u32,
    registry: &super::registration::NiFactoryRegistry,
) -> Result<StreamObjectTable, StreamObjectError> {
    use StreamObjectErrorCode as Code;
    if version < 0x0500_0001 {
        return Err(table_error(
            Code::UnsupportedVersion,
            0,
            None,
            None,
            String::new(),
        ));
    }
    let u16_at = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|raw| u16::from_le_bytes([raw[0], raw[1]]))
    };
    let mut position = 0;
    let Some(type_count) = u16_at(position) else {
        return Err(table_error(
            Code::TruncatedTable,
            position,
            None,
            None,
            String::new(),
        ));
    };
    position += 2;
    let mut result = StreamObjectTable::default();
    for type_index in 0..usize::from(type_count) {
        let length_offset = position;
        let Some(raw) = bytes.get(position..position + 4) else {
            return Err(table_error(
                Code::TruncatedTable,
                length_offset,
                Some(type_index),
                None,
                String::new(),
            ));
        };
        position += 4;
        let length = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if length > 255 {
            return Err(table_error(
                Code::NameTooLong,
                length_offset,
                Some(type_index),
                None,
                String::new(),
            ));
        }
        let Some(raw_name) = bytes.get(position..position + length) else {
            return Err(table_error(
                Code::TruncatedTable,
                position,
                Some(type_index),
                None,
                String::new(),
            ));
        };
        let name_bytes = raw_name.split(|&byte| byte == 0).next().unwrap_or(raw_name);
        let name: String = name_bytes.iter().map(|&byte| char::from(byte)).collect();
        position += length;
        let Some(original_constructor) = registry.find(&name) else {
            return Err(table_error(
                Code::MissingRegistration,
                position,
                Some(type_index),
                None,
                name,
            ));
        };
        result.types.push(StreamTypeEntry {
            name,
            original_constructor,
        });
    }
    if object_count as usize > (bytes.len() - position) / 2 {
        return Err(table_error(
            Code::TruncatedTable,
            position,
            None,
            None,
            String::new(),
        ));
    }
    let mut indices = Vec::with_capacity(object_count as usize);
    for object_index in 0..object_count as usize {
        let index_offset = position;
        let index = u16_at(position).expect("the count was checked against the bytes");
        position += 2;
        if usize::from(index) >= result.types.len() {
            return Err(table_error(
                Code::InvalidTypeIndex,
                index_offset,
                Some(usize::from(index)),
                Some(object_index),
                String::new(),
            ));
        }
        indices.push(index);
    }
    for (object_index, &type_index) in indices.iter().enumerate() {
        let name = result.types[usize::from(type_index)].name.clone();
        let created = registry.create(world, &name);
        match created {
            super::registration::FactoryCreate::Created {
                requested_name,
                original_constructor,
                entity,
            } => {
                result.objects.push(StreamObjectEntry {
                    type_index,
                    requested_name,
                    original_constructor,
                    entity,
                });
            }
            super::registration::FactoryCreate::Unregistered => {
                despawn_created(world, &result);
                let index = Some(usize::from(type_index));
                return Err(table_error(
                    Code::MissingRegistration,
                    position,
                    index,
                    Some(object_index),
                    name,
                ));
            }
            super::registration::FactoryCreate::Unsupported => {
                despawn_created(world, &result);
                let index = Some(usize::from(type_index));
                return Err(table_error(
                    Code::UnsupportedFactory,
                    position,
                    index,
                    Some(object_index),
                    name,
                ));
            }
        }
    }
    result.bytes_consumed = position;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::registration::NiFactoryRegistry;
    use super::*;

    fn name(bytes: &mut Vec<u8>, text: &str) {
        bytes.extend((text.len() as u32).to_le_bytes());
        bytes.extend(text.as_bytes());
    }

    fn table(names: &[&str], indices: &[u16]) -> Vec<u8> {
        let mut bytes = (names.len() as u16).to_le_bytes().to_vec();
        for text in names {
            name(&mut bytes, text);
        }
        for index in indices {
            bytes.extend(index.to_le_bytes());
        }
        bytes
    }

    /// Ported from `gamebryo_stream_objects`.
    #[test]
    fn repeated_types_and_aliases_create_separate_objects() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiFloatData", 0x006E_3440);
        registry.insert_or_assign("FloatAlias", 0x006E_3440);
        let mut bytes = table(&["FloatAlias", "NiFloatData"], &[0, 0, 1]);
        let consumed = bytes.len();
        bytes.push(0xCC);
        let result =
            read_stream_object_table(&mut world, &bytes, 3, 0x0500_0001, &registry).unwrap();
        assert_eq!(result.types.len(), 2);
        assert_eq!(result.objects.len(), 3);
        assert_eq!(result.bytes_consumed, consumed);
        assert_eq!(result.types[0].name, "FloatAlias");
        assert_eq!(result.types[1].name, "NiFloatData");
        assert_eq!(result.objects[0].type_index, 0);
        assert_eq!(result.objects[2].type_index, 1);
        assert_eq!(result.objects[0].requested_name, "FloatAlias");
        assert_eq!(result.objects[2].requested_name, "NiFloatData");
        assert_eq!(result.objects[0].original_constructor, 0x006E_3440);
        assert_ne!(result.objects[0].entity, result.objects[1].entity);
        assert_ne!(result.objects[1].entity, result.objects[2].entity);
        assert_ne!(result.objects[0].entity, result.objects[2].entity);
        assert_eq!(
            world.get::<NiClass>(result.objects[1].entity),
            Some(&NiClass::FloatKeyData)
        );
    }

    /// An unused missing name still fails; an unused unsupported class does not.
    #[test]
    fn unused_types_follow_the_original_rules() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiFloatData", 0x006E_3440);
        registry.insert_or_assign("NiCamera", 0x0070_D730);
        let bytes = table(&["NiFloatData", "AbsentUnusedType"], &[0]);
        let missing =
            read_stream_object_table(&mut world, &bytes, 1, 0x0500_0001, &registry).unwrap_err();
        assert_eq!(missing.code, StreamObjectErrorCode::MissingRegistration);
        assert_eq!(missing.original_error_code(), Some(5));
        assert_eq!(missing.type_index, Some(1));
        assert_eq!(missing.name, "AbsentUnusedType");

        let bytes = table(&["NiFloatData", "NiCamera"], &[0]);
        let unused =
            read_stream_object_table(&mut world, &bytes, 1, 0x0500_0001, &registry).unwrap();
        assert_eq!(unused.objects.len(), 1);
        let bytes = table(&["NiFloatData", "NiCamera"], &[1]);
        let used =
            read_stream_object_table(&mut world, &bytes, 1, 0x0500_0001, &registry).unwrap_err();
        assert_eq!(used.code, StreamObjectErrorCode::UnsupportedFactory);
        assert_eq!(used.object_index, Some(0));
        assert_eq!(used.name, "NiCamera");
    }

    #[test]
    fn preflight_rejects_before_any_factory_runs() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiFloatData", 0x006E_3440);
        let code = |world: &mut World, bytes: &[u8], count, version| {
            read_stream_object_table(world, bytes, count, version, &registry)
                .unwrap_err()
                .code
        };
        let mut bytes = table(&["NiFloatData"], &[1]);
        assert_eq!(
            code(&mut world, &bytes, 1, 0x0500_0001),
            StreamObjectErrorCode::InvalidTypeIndex
        );
        assert_eq!(
            read_stream_object_table(&mut world, &bytes, 1, 0x0500_0001, &registry)
                .unwrap_err()
                .object_index,
            Some(0)
        );
        assert_eq!(
            code(&mut world, &bytes, 1, 0x0500_0000),
            StreamObjectErrorCode::UnsupportedVersion
        );
        bytes.pop();
        assert_eq!(
            code(&mut world, &bytes, 1, 0x0500_0001),
            StreamObjectErrorCode::TruncatedTable
        );
        assert_eq!(
            code(&mut world, &bytes, 2, 0x0500_0001),
            StreamObjectErrorCode::TruncatedTable
        );
        let mut overlong = 1u16.to_le_bytes().to_vec();
        overlong.extend(256u32.to_le_bytes());
        assert_eq!(
            code(&mut world, &overlong, 0, 0x0500_0001),
            StreamObjectErrorCode::NameTooLong
        );
        assert_eq!(
            code(&mut world, &overlong[..5], 0, 0x0500_0001),
            StreamObjectErrorCode::TruncatedTable
        );
        let mut short_name = 1u16.to_le_bytes().to_vec();
        short_name.extend(4u32.to_le_bytes());
        short_name.push(b'N');
        assert_eq!(
            code(&mut world, &short_name, 0, 0x0500_0001),
            StreamObjectErrorCode::TruncatedTable
        );
        assert_eq!(
            world.query::<&NiClass>().iter(&world).count(),
            0,
            "no factory ran"
        );
        let empty =
            read_stream_object_table(&mut world, &[0, 0], 0, 0x0500_0001, &registry).unwrap();
        assert!(empty.types.is_empty() && empty.objects.is_empty());
        assert_eq!(empty.bytes_consumed, 2);
    }

    /// Host rule: dropping the C++ table on a failed factory call drops what it made so far.
    #[test]
    fn a_factory_failure_midway_leaves_nothing() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiFloatData", 0x006E_3440);
        registry.insert_or_assign("NiCamera", 0x0070_D730);
        let bytes = table(&["NiFloatData", "NiCamera"], &[0, 0, 1]);
        let error =
            read_stream_object_table(&mut world, &bytes, 3, 0x0500_0001, &registry).unwrap_err();
        assert_eq!(error.code, StreamObjectErrorCode::UnsupportedFactory);
        assert_eq!(error.object_index, Some(2));
        assert_eq!(world.query::<&NiClass>().iter(&world).count(), 0);
    }
}
