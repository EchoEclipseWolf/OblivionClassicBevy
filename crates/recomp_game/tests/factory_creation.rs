//! Ported from the C++ `tests/gamebryo/factory_creation_tests.cpp`.

use bevy_ecs::prelude::*;
use recomp_game::gamebryo::animation::{FloatInterpolator, FloatKeyData};
use recomp_game::gamebryo::color_animation::ColorKeyData;
use recomp_game::gamebryo::particle_color::ParticleColorModifier;
use recomp_game::gamebryo::particle_drag::ParticleDragModifier;
use recomp_game::gamebryo::particle_dynamics::{
    ParticleGrowFadeModifier, ParticlePositionModifier,
};
use recomp_game::gamebryo::position_animation::PositionKeyData;
use recomp_game::gamebryo::registration::{FactoryCreate, NiFactoryRegistry};

fn created(result: FactoryCreate) -> (String, u32, Entity) {
    match result {
        FactoryCreate::Created {
            requested_name,
            original_constructor,
            entity,
        } => (requested_name, original_constructor, entity),
        other => panic!("not created: {other:?}"),
    }
}

/// 00712590's lookup and the 006E3440 / 006D2DD0 / 006E4520 cold factories: typed status, address-keyed aliases and
/// independent owners.
#[test]
fn creation() {
    let mut world = World::new();
    let mut registry = NiFactoryRegistry::default();
    assert_eq!(
        registry.create(&mut world, "NiFloatData"),
        FactoryCreate::Unregistered
    );
    assert_eq!(registry.find("NiFloatData"), None);

    registry.insert_or_assign("NiCamera", 0x0070_D730);
    assert_eq!(
        registry.create(&mut world, "NiCamera"),
        FactoryCreate::Unsupported,
        "a registered unsupported factory is not invoked"
    );
    assert_eq!(registry.find("NiCamera"), Some(0x0070_D730));

    registry.insert_or_assign("NiFloatData", 0x006E_3440);
    registry.insert_or_assign("NiFloatInterpolator", 0x006D_2DD0);
    registry.insert_or_assign("SyntheticFloatAlias", 0x006E_3440);
    registry.insert_or_assign("NiColorData", 0x006E_4520);
    registry.insert_or_assign("NiPSysColorModifier", 0x0075_A360);
    registry.insert_or_assign("NiPSysPositionModifier", 0x0075_5690);
    registry.insert_or_assign("NiPSysGrowFadeModifier", 0x0075_6DA0);
    registry.insert_or_assign("NiPSysDragModifier", 0x0075_8960);
    registry.insert_or_assign("NiPosData", 0x006D_9D40);
    let (_, data_constructor, data) = created(registry.create(&mut world, "NiFloatData"));
    let (alias_name, alias_constructor, alias) =
        created(registry.create(&mut world, "SyntheticFloatAlias"));
    let (_, _, interpolator) = created(registry.create(&mut world, "NiFloatInterpolator"));
    let (_, color_constructor, color) = created(registry.create(&mut world, "NiColorData"));
    let (_, modifier_constructor, modifier) =
        created(registry.create(&mut world, "NiPSysColorModifier"));
    let (_, position_constructor, position) =
        created(registry.create(&mut world, "NiPSysPositionModifier"));
    let (_, grow_constructor, grow) =
        created(registry.create(&mut world, "NiPSysGrowFadeModifier"));
    let (_, drag_constructor, drag) = created(registry.create(&mut world, "NiPSysDragModifier"));
    let (_, position_data_constructor, position_data) =
        created(registry.create(&mut world, "NiPosData"));

    assert_eq!(data_constructor, 0x006E_3440);
    assert!(world.get::<FloatKeyData>(data).unwrap().keys().is_empty());
    assert_eq!(alias_name, "SyntheticFloatAlias");
    assert_eq!(alias_constructor, data_constructor);
    assert!(world.get::<FloatKeyData>(alias).is_some());
    assert_ne!(alias, data, "an alias has an independent owner");
    assert_eq!(
        world
            .get::<FloatInterpolator>(interpolator)
            .unwrap()
            .data_10,
        None
    );
    assert_eq!(color_constructor, 0x006E_4520);
    assert!(world.get::<ColorKeyData>(color).unwrap().keys().is_empty());
    assert_eq!(modifier_constructor, 0x0075_A360);
    assert_eq!(
        world
            .get::<ParticleColorModifier>(modifier)
            .unwrap()
            .data_18,
        None
    );
    assert_eq!(position_constructor, 0x0075_5690);
    assert!(
        world
            .get::<ParticlePositionModifier>(position)
            .unwrap()
            .base
            .active_14
    );
    assert_eq!(grow_constructor, 0x0075_6DA0);
    assert_eq!(
        world
            .get::<ParticleGrowFadeModifier>(grow)
            .unwrap()
            .grow_time_18,
        0.0
    );
    assert_eq!(drag_constructor, 0x0075_8960);
    assert!(
        !world
            .get::<ParticleDragModifier>(drag)
            .unwrap()
            .drag_object_linked
    );
    assert_eq!(position_data_constructor, 0x006D_9D40);
    assert!(
        world
            .get::<PositionKeyData>(position_data)
            .unwrap()
            .keys()
            .is_empty()
    );

    assert_eq!(registry.find("NiFloatData"), Some(0x006E_3440));
    assert_eq!(registry.entries().len(), 10);
    registry.insert_or_assign("NiFloatData", 0x0070_D730);
    assert_eq!(
        registry.create(&mut world, "NiFloatData"),
        FactoryCreate::Unsupported,
        "binding follows the registered address, not the class name"
    );
    created(registry.create(&mut world, "SyntheticFloatAlias"));
    registry.erase("SyntheticFloatAlias");
    assert_eq!(
        registry.create(&mut world, "SyntheticFloatAlias"),
        FactoryCreate::Unregistered
    );
    assert!(world.get::<FloatKeyData>(alias).unwrap().keys().is_empty());
    registry.release_storage();
    assert!(registry.entries().is_empty());
}
