//! `00667BE0` PlayerCharacter_Load3D (vt+0x14C): the player's outfit and skill thresholds, the first-person skeleton under
//! "Player1stPerson" with its own skin info and animation data, then Character_Load3D for the third person.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::TesObjectRefr;
use super::character_3d::{SkinBone, SkinInfo, SkinSlot, SkinSlotScratch};
use super::player::PlayerCharacter;
use crate::bs::model_loader::{ModelBuildServices, load_model_now, object_by_name};
use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child};
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::scene_node::{NiAvWorld, SceneNodePayload, spawn_scene_node};

/// IDA: 00B36BB8 sFirstPersonSkeleton.
const FIRST_PERSON_SKELETON_B36BB8: u32 = 0x00B3_6BB8;

// IDA: N/A a float game setting's value.
fn setting_float(world: &World, address: u32) -> Result<f32, StreamError> {
    world
        .resource::<super::game_settings::GameSettingCollection>()
        .raw_at(address)
        .map(f32::from_bits)
        .ok_or(StreamError(
            "006670C0: a skill-use setting isn't registered",
        ))
}

/// IDA: 00548030(skill, specialized, major): pow(f32(skill * 00B37D98 fSkillUseFactor), 00B37D90 fSkillUseExp) (the CRT's
/// pow, stored as a float) times 00B37670 fSkillUseSpecMult (else 1.0) times 00B37678 fSkillUseMajorMult (else 00B37680
/// fSkillUseMinorMult), each product at 24 bits. Completion: 100%.
fn skill_use_requirement(
    world: &World,
    skill: i32,
    specialized: bool,
    major: bool,
) -> Result<f32, StreamError> {
    let specialization = if specialized {
        setting_float(world, 0x00B3_7670)?
    } else {
        1.0
    };
    let multiplier = setting_float(world, if major { 0x00B3_7678 } else { 0x00B3_7680 })?;
    let scaled = skill as f32 * setting_float(world, 0x00B3_7D98)?;
    let powered = f64::from(scaled).powf(f64::from(setting_float(world, 0x00B3_7D90)?)) as f32;
    Ok(powered * specialization * multiplier)
}

/// IDA: 006670C0 PlayerCharacter_InitSkillRequirements: for each of the 21 skills 00663C50: the DataHandler's skill (+0xD8),
/// the player's floored skill (005F1910 on its +0x2C), and unless the class is iClassCharactergenClass (00B37D00), whether the
/// skill's +0x34 specialization is the class's +0x40 and whether it's a major skill (0051C090); 00548030's requirement, 0.0
/// taking 1.0, into +0x7A4 + 4 * index. Completion: 100%.
/// # Errors
/// A setting or class missing.
pub fn init_skill_requirements(world: &mut World, player: Entity) -> Result<(), StreamError> {
    let skills = world
        .get_resource::<super::data_handler::DataHandler>()
        .map(|handler| handler.skills_d8)
        .ok_or(StreamError("00663C81: no DataHandler"))?;
    let class = world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .and_then(|base| world.get::<super::forms_actors::TesNpc>(base))
        .and_then(|npc| match npc.class_104 {
            super::form_components::FormRef::Resolved(class) => class,
            super::form_components::FormRef::Raw(_) => None,
        })
        .ok_or(StreamError("00663CAF: a player with no class faults"))?;
    let placeholder = world
        .resource::<super::game_settings::GameSettingCollection>()
        .raw_at(0x00B3_7D00)
        .ok_or(StreamError(
            "00663CB2: iClassCharactergenClass isn't registered",
        ))?;
    let charactergen = world
        .get::<super::form::FormIdentity>(class)
        .expect("a form")
        .form_id_0c
        == placeholder;
    let mut required = [0_u32; 21];
    for (index, &skill) in skills.iter().enumerate() {
        let Some(data) = world.get::<super::data_handler::TesSkill>(skill) else {
            continue;
        };
        let code = data.skill_2c;
        let specialization = data.specialization_34;
        let level = super::player::floor_calculated_actor_value(world, player, code as u32)?;
        let (specialized, major) = if charactergen {
            (false, false)
        } else {
            let class_data = world
                .get::<super::forms_misc::TesClass>(class)
                .expect("a class");
            (
                class_data.specialization_40 == specialization,
                class_data.major_skills_44.contains(&(code as u32)),
            )
        };
        let mut value = skill_use_requirement(world, level, specialized, major)?;
        if value == 0.0 {
            value = 1.0;
        }
        required[index] = value.to_bits();
    }
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .required_skill_exp_7a4 = required;
    Ok(())
}

/// IDA: 00667BE0 PlayerCharacter_Load3D (vt+0x14C): outside a load (+0x18 bit 2), TESNPC_AutoEquip(base, player, 1, 1, 0, 0)
/// (005227A0) and 006670C0. ModelLoader_LoadModelNow(sFirstPersonSkeleton, 1, 3, 1); a new NiNode (0070A8A0) with +0x18 |=
/// 0x40 is +0x5D0, named "Player1stPerson", the loaded skeleton attached to it (vt+0x84, 1); "Camera01" (00B3BB0C) and "Bip"
/// (00B3BB14, the 4-byte inline string at 00A738A4) found by name ("Missing 'Camera01' on '%s'." and "Missing 'Bip' on '%s'." through DebugLog_Printf otherwise);
/// NiAVObject_Update(0.0, 1); +0x5D4 = Camera01's world z (+0x90), the skeleton's translate (0, 0, -that) and its rotation
/// 004D7C50(identity, 0); the first-person "Quiver" culled (+0x18 |= 1); +0x5C8 a new skin info on the node (00478730 with
/// 00478070), +0x5CC a new ActorAnimData (00473EB0); then with +0x588 set to 1 Character_Load3D (00659F30), +0x588 restored.
/// The first-person KF list (IdleAnimRoot_FindKFFiles) into 00475D80 on +0x5CC; the process's vt+0xEC/0x104 and both
/// vt+0x114 calls; the third person named "Player", +0x18 |= 1, and its "Camera01" in 00B3BB10 (the base's model path in the
/// warning). 00578CF0's reticle update (queued: `ReticleRequests`), 005E4DD0, the vampirism test (a vampire is a stop),
/// vt+0x31C(0) and 005EA1A0 on the first-person skeleton. vt+0x148 (0065B750 with the controller Phase 11 made: its phantom's
/// 3D, the cell's Havok world, 0088D070) and the controller's vt+0x90 are a stop. Completion: through 00667FBE.
/// # Errors
/// A stop above, or one inside the calls.
pub fn player_load_3d(
    world: &mut World,
    player: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, StreamError> {
    if super::forms_references::save_load_flags(world)? & 2 == 0 {
        super::actor_enable::auto_equip(world, player, true, true, false)?;
        init_skill_requirements(world, player)?;
    }
    let path = world
        .resource::<super::game_settings::GameSettingCollection>()
        .text_at(FIRST_PERSON_SKELETON_B36BB8)
        .ok_or(StreamError("00667C0A: sFirstPersonSkeleton has no text"))?;
    let skeleton = load_model_now(world, &path, true, 3, true, services)?.ok_or(StreamError(
        "00667CCB: a first-person skeleton that didn't load faults",
    ))?;
    let node = spawn_scene_node(world, SceneNodePayload::default());
    world
        .get_mut::<SceneNodePayload>(node)
        .expect("a NiNode")
        .effective_av_flags |= 0x40;
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .first_person_node_5d0 = Some(node);
    world.get_mut::<NiObjectNet>(node).expect("a NiNode").name = Some(b"Player1stPerson".to_vec());
    if attach_scene_child(world, node, Some(skeleton), true) != AttachChildStatus::Attached {
        return Err(StreamError("00667C55: attaching the first-person skeleton"));
    }
    let camera = object_by_name(world, node, b"Camera01");
    let bip = object_by_name(world, node, b"Bip");
    let text = String::from_utf8_lossy(&path).into_owned();
    if camera.is_none() {
        recomp_core::logging::emit(
            recomp_core::logging::Level::Warning,
            &format!("Missing 'Camera01' on '{text}'."),
        );
    }
    if bip.is_none() {
        recomp_core::logging::emit(
            recomp_core::logging::Level::Warning,
            &format!("Missing 'Bip' on '{text}'."),
        );
    }
    world
        .get_resource_or_init::<FirstPersonNodes>()
        .camera_b3bb0c = camera;
    world.get_resource_or_init::<FirstPersonNodes>().bip_b3bb14 = bip;
    super::reference_3d::update_with_controllers(world, node, 0.0)?;
    let camera = camera.ok_or(StreamError("00667CC2: a null Camera01 faults"))?;
    let eye = world
        .get::<NiAvWorld>(camera)
        .map(|world_state| world_state.transform.translation[2])
        .ok_or(StreamError("00667CC2: Camera01 with no world transform"))?;
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .camera_eye_height_offset_5d4 = eye;
    let rotation = super::biped_attach::race_scaled_rotation(
        world,
        player,
        &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        false,
    )?;
    {
        let mut payload = world
            .get_mut::<SceneNodePayload>(skeleton)
            .ok_or(StreamError(
                "00667CCB: a first-person skeleton that isn't a NiNode",
            ))?;
        payload.local_transform.translation = [0.0, 0.0, -eye];
        payload.local_transform.rotation = rotation;
    }
    if let Some(quiver) = object_by_name(world, node, b"Quiver")
        && let Some(mut payload) = world.get_mut::<SceneNodePayload>(quiver)
    {
        payload.effective_av_flags |= 1;
    }
    // 00667D2E 00478730(player, node): the 0x154 bytes and 00B33C80 zeroed, +0x150 the player, 00478070 on the node.
    world.get_resource_or_init::<SkinSlotScratch>().slots_b33c80 = [SkinSlot::default(); 16];
    let skin = world
        .spawn(SkinInfo {
            bip01_00: None,
            bones_04: [SkinBone::default(); 9],
            slots_4c: [SkinSlot::default(); 16],
            owner_150: player,
        })
        .id();
    super::character_3d::skin_info_find_bones(world, skin, node)?;
    let anim = world
        .spawn(super::actor_animation::ActorAnimData::default())
        .id();
    {
        let mut data = world
            .get_mut::<PlayerCharacter>(player)
            .expect("the player");
        data.first_person_skin_info_5c8 = Some(skin);
        data.first_person_anim_data_5cc = Some(anim);
    }
    let third_person = world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .view_flags_588[0];
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .view_flags_588[0] = 1;
    let result = super::reference_3d::load_3d_body(world, player, services);
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .view_flags_588[0] = third_person;
    let third = result?.ok_or(StreamError(
        "00667EFD: a null third-person node faults at its name",
    ))?;
    // 00667E8C: IdleAnimRoot_FindKFFiles(sFirstPersonSkeleton, 0) into 00475D80 on +0x5CC, the new node and the player; its
    // answer is unused.
    let files = super::forms_ai::root_kf_files(world, &path)?;
    super::actor_animation::init_from_kf_list(world, anim, files, Some(node), player, services)?;
    // 00667EA5: the process's vt+0xEC(1, 0) into vt+0x104, vt+0x114 with the third person's anim data (TESObjectREFR_GetAnimData),
    // then vt+0x114 with +0x5CC. With no weapon entry both vt+0x114 leave +0x110 null; a weapon stops in the first.
    let third_anim = super::ai_conditions::process_of(world, player)
        .and_then(|process| world.get::<super::actor_process::MiddleHighProcess>(process))
        .and_then(|middle| middle.anim_data_17c);
    super::actor_enable::process_animation_nodes(world, player, third_anim)?;
    // 00667EFD: the third person is named "Player" and culled (+0x18 |= 1); 00B3BB10 its "Camera01".
    world
        .get_mut::<NiObjectNet>(third)
        .ok_or(StreamError(
            "00667EFD: a third-person node that isn't a NiObjectNET",
        ))?
        .name = Some(b"Player".to_vec());
    world
        .get_mut::<SceneNodePayload>(third)
        .ok_or(StreamError(
            "00667F07: a third-person node that isn't a NiNode",
        ))?
        .effective_av_flags |= 1;
    let third_camera = object_by_name(world, third, b"Camera01");
    world
        .get_resource_or_init::<FirstPersonNodes>()
        .third_person_camera_b3bb10 = third_camera;
    if third_camera.is_none() {
        let base = world
            .get::<TesObjectRefr>(player)
            .and_then(|refr| refr.base_form_1c);
        let model = base
            .and_then(|base| super::reference_load::model_path_of(world, base))
            .unwrap_or_default();
        recomp_core::logging::emit(
            recomp_core::logging::Level::Warning,
            &format!(
                "Missing 'Camera01' on '{}'.",
                String::from_utf8_lossy(&model)
            ),
        );
    }
    // 00667F3F 00578CF0 InterfaceManager_UpdateReticleIfPlayerHas3D(1): the player's vt+0x154 (006600F0: the third person's node
    // while +0x588 is set or +0x5D0 is null, else +0x5D0) nonnull runs 0057D940(1). Host substitution: the drain holds no
    // interface context, so the request is queued and the driver runs it when its drain returns.
    if player_ni_node(world, player).is_some() {
        world
            .get_resource_or_init::<super::hud_reticle::ReticleRequests>()
            .modes
            .push(1);
    }
    // 00667F4B 005E4DD0 Actor_QueueMagicResources.
    super::character_3d::queue_magic_resources(
        world,
        player,
        &mut super::reference_load::ReferenceLoadServices {
            archives: services.archives.as_deref_mut(),
        },
    )?;
    // 00667F50: vt+0x284(0x45), vampirism, nonzero: the process's vt+0x31C(1), vt+0x5C, 00B33D80 = 1 around vt+0x318(player).
    if super::ai_process::actor_get_actor_value(world, player, 0x45)? != 0 {
        return Err(StreamError(
            "00667F63: a vampire player's equipment 3D (0063CDC0 under 00B33D80)",
        ));
    }
    let process = super::ai_conditions::process_of(world, player)
        .ok_or(StreamError("00667F98: the player with no process faults"))?;
    // 00667F98 vt+0x31C(0) (006295E0: +0x16C).
    world
        .get_mut::<super::actor_process::MiddleHighProcess>(process)
        .ok_or(StreamError(
            "00667F98: the player's process with no MiddleHigh part",
        ))?
        .equipment_update_pending_16c = 0;
    // 00667FAE 005EA1A0 Actor_UpdateSkinProperty(player, the first-person skeleton).
    super::actor_enable::find_skin_property(world, process, skeleton)?;
    Err(StreamError(
        "00667FBE: vt+0x148 0065B750 with the player's controller (its phantom's 3D, the cell's Havok world, 0088D070) and the \
         controller's vt+0x90",
    ))
}

/// IDA: 006600F0 PlayerCharacter_GetNiNode (vt+0x154): +0x588 set or +0x5D0 null gives 00612220's node (+0x3C), else +0x5D0.
/// Completion: 100%.
#[must_use]
pub fn player_ni_node(world: &World, player: Entity) -> Option<Entity> {
    let data = world.get::<PlayerCharacter>(player)?;
    let third = world
        .get::<super::reference_3d::Reference3D>(player)
        .map(|loaded| loaded.0);
    if data.view_flags_588[0] != 0 {
        return third;
    }
    data.first_person_node_5d0.or(third)
}

/// IDA: 00B3BB0C and 00B3BB14, the first-person skeleton's "Camera01" and "Bip" (00667BE0), and 00B3BB10, the third person's
/// "Camera01".
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FirstPersonNodes {
    pub camera_b3bb0c: Option<Entity>,
    pub bip_b3bb14: Option<Entity>,
    pub third_person_camera_b3bb10: Option<Entity>,
}
