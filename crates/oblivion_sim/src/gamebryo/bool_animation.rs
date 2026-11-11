//! NiBoolData (also NiVisData), NiBoolInterpolator and NiBoolTimelineInterpolator.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

/// IDA: the sentinel time in the bool interpolators (-FLT_MAX).
pub const INVALID_BOOL_INTERPOLATOR_TIME: f32 = -f32::MAX;
/// IDA: the "no value" byte in the bool interpolators.
pub const INVALID_BOOL_INTERPOLATOR_VALUE: u8 = 2;

/// The key type, kept raw so that an unregistered value stays representable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoolKeyType(pub u8);

impl BoolKeyType {
    pub const NONE: Self = Self(0);
    pub const LINEAR: Self = Self(1);
    pub const STEP: Self = Self(5);
}

/// Host-owned key representation; the original step payload uses a raw byte.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoolKey {
    pub time: f32,
    pub value: u8,
}

/// IDA: 006E8740 NiBoolData factory. Cold: no keys, type 0.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct BoolKeyData {
    key_type: BoolKeyType,
    keys: Vec<BoolKey>,
}

impl BoolKeyData {
    // IDA: N/A a validated host key owner over 006E8740's cold fields
    // Completion: 100% for finite sorted input, raw byte retained
    // TODO: [ ] Recover original malformed-key behaviour.
    pub fn new(key_type: BoolKeyType, keys: Vec<BoolKey>) -> Result<Self, GameError> {
        if (key_type != BoolKeyType::LINEAR && key_type != BoolKeyType::STEP) || keys.is_empty() {
            return Err(GameError::unclassified(
                0,
                "unsupported or empty bool key data",
            ));
        }
        for (index, key) in keys.iter().enumerate() {
            if !key.time.is_finite() || (index != 0 && key.time <= keys[index - 1].time) {
                return Err(GameError::unclassified(
                    0,
                    "bool key times must be finite and strictly sorted",
                ));
            }
        }
        Ok(Self { key_type, keys })
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn key_type(&self) -> BoolKeyType {
        self.key_type
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn keys(&self) -> &[BoolKey] {
        &self.keys
    }

    // IDA: 006BDBA0 key scan and 006C1C60 type-5 step dispatch
    // Completion: 100% on finite in-range step tracks and the singleton and sentinel branches
    // TODO: [ ] Recover original out-of-range and malformed behaviour; type 1 is collapse-only (its slot is null).
    pub fn sample(&self, time: f32, cached_index: &mut u32) -> Option<u8> {
        let keys = &self.keys;
        if keys.is_empty() {
            return None;
        }
        if keys.len() == 1 || time == INVALID_BOOL_INTERPOLATOR_TIME {
            return Some(keys[0].value);
        }
        if self.key_type != BoolKeyType::STEP
            || !time.is_finite()
            || time < keys[0].time
            || time > keys[keys.len() - 1].time
        {
            return None;
        }
        let mut first = if (*cached_index as usize) < keys.len() - 1 {
            *cached_index as usize
        } else {
            0
        };
        if keys[first].time > time {
            first = 0;
        }
        while first + 1 < keys.len() - 1 && keys[first + 1].time < time {
            first += 1;
        }
        let left = &keys[first];
        let right = &keys[first + 1];
        let ratio = (time - left.time) / (right.time - left.time);
        *cached_index = first as u32;
        Some(if ratio >= 1.0 {
            right.value
        } else {
            left.value
        })
    }
}

/// IDA: 006E8370 NiBoolInterpolator via 006EC220 / 006EBA00: the cold scalar cache state.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BoolInterpolator {
    pub last_time_08: f32,
    pub cached_value_0c: u8,
    pub data_10: Option<Entity>,
    pub last_key_index: u32,
}

impl Default for BoolInterpolator {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            last_time_08: INVALID_BOOL_INTERPOLATOR_TIME,
            cached_value_0c: INVALID_BOOL_INTERPOLATOR_VALUE,
            data_10: None,
            last_key_index: 0,
        }
    }
}

impl BoolInterpolator {
    // IDA: N/A host owner transfer
    // Completion: 100% of the replacement and invalidation policy
    // TODO: [ ] Verify original SetData cache invalidation and refcount operations.
    pub fn set_data(&mut self, data: Option<Entity>) {
        self.data_10 = data;
        self.last_time_08 = INVALID_BOOL_INTERPOLATOR_TIME;
        self.cached_value_0c = INVALID_BOOL_INTERPOLATOR_VALUE;
        self.last_key_index = 0;
    }

    // IDA: 006E8550 LoadBinary writes raw byte +0x0C, then retains data +0x10
    // Completion: 100% of the host scalar and owner assignment, without changing last time or key index
    // TODO: [ ] Compare original refcount timing on already populated objects.
    pub fn load_scalar_and_data(&mut self, value: u8, data: Option<Entity>) {
        self.cached_value_0c = value;
        self.data_10 = data;
    }

    // IDA: 006E8230 Update(float, unused, bool*)
    // Completion: 100% of the invalid-output and cache branches. `data` is what `data_10` links to.
    // TODO: [ ] Compare original runtime behaviour on populated animation tracks.
    pub fn update(&mut self, data: Option<&BoolKeyData>, time: f32, output: &mut bool) -> bool {
        if time == self.last_time_08 {
            if self.cached_value_0c == INVALID_BOOL_INTERPOLATOR_VALUE {
                *output = false;
                return false;
            }
            *output = self.cached_value_0c != 0;
            return true;
        }
        if let Some(data) = data.filter(|data| !data.keys().is_empty()) {
            if let Some(value) = data.sample(time, &mut self.last_key_index) {
                self.cached_value_0c = u8::from(value != 0);
            } else {
                *output = false;
                return false;
            }
        }
        if self.cached_value_0c == INVALID_BOOL_INTERPOLATOR_VALUE {
            *output = false;
            return false;
        }
        *output = self.cached_value_0c != 0;
        self.last_time_08 = time;
        true
    }

    // IDA: 006E8000 Collapse
    // Completion: 100% of the zero, singleton and equal type-1-or-5 branches
    // TODO: [ ] Recover original reference-count and unsupported key layout paths.
    pub fn collapse(&mut self, data: Option<&BoolKeyData>) {
        let Some(data) = data else {
            return;
        };
        let keys = data.keys();
        if keys.is_empty() {
            self.data_10 = None;
            self.cached_value_0c = INVALID_BOOL_INTERPOLATOR_VALUE;
            return;
        }
        if keys.len() > 1 {
            if data.key_type() != BoolKeyType::LINEAR && data.key_type() != BoolKeyType::STEP {
                return;
            }
            if keys.iter().any(|key| key.value != keys[0].value) {
                return;
            }
        }
        self.cached_value_0c = keys[0].value;
        self.data_10 = None;
    }
}

/// IDA: 006E7E70 NiBoolTimelineInterpolator through 006E7F50: the inherited state and two added fields.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct BoolTimelineInterpolator {
    pub base: BoolInterpolator,
    pub previous_key_index: u32,
    pub previous_raw_value: u8,
}

impl BoolTimelineInterpolator {
    // IDA: N/A host assignment of 006E7C20's two added fields
    // Completion: 100% of the fresh host owner and cache state
    // TODO: [ ] Recover original timeline SetData behaviour.
    pub fn set_timeline_data(&mut self, data: Option<Entity>) {
        self.base.set_data(data);
        self.previous_key_index = 0;
        self.previous_raw_value = 0;
    }

    // IDA: 006E7C20 timeline Update with 006BDBA0 and 006C1C60 keyed sampling
    // Completion: 100% of the bounded forward and wrap skipped-key scan on validated step tracks
    // TODO: [ ] Recover original behaviour outside finite sorted key ranges.
    pub fn update_timeline(
        &mut self,
        data: Option<&BoolKeyData>,
        time: f32,
        output: &mut bool,
    ) -> bool {
        if time == self.base.last_time_08 {
            if self.base.cached_value_0c == INVALID_BOOL_INTERPOLATOR_VALUE {
                *output = false;
                return false;
            }
            *output = self.base.cached_value_0c != 0;
            return true;
        }
        if let Some(data) = data.filter(|data| !data.keys().is_empty()) {
            let old_index = self.base.last_key_index;
            let Some(raw) = data.sample(time, &mut self.base.last_key_index) else {
                *output = false;
                return false;
            };
            let last_index = self.base.last_key_index;
            self.previous_key_index = old_index;
            let mut emitted = raw;
            if raw == self.previous_raw_value {
                let keys = data.keys();
                let opposite = u8::from(raw == 0);
                let contains_opposite = |first: usize, last: usize| {
                    (first..last).any(|index| keys[index].value == opposite)
                };
                let old = old_index as usize;
                let new = last_index as usize;
                // The C++ `old == new || old + 1 <= new`.
                if old <= new {
                    if contains_opposite(old + 1, new) {
                        emitted = opposite;
                    }
                } else if contains_opposite(old + 1, keys.len()) || contains_opposite(0, new) {
                    emitted = opposite;
                }
            }
            self.base.cached_value_0c = u8::from(emitted != 0);
            self.previous_raw_value = raw;
        }
        if self.base.cached_value_0c == INVALID_BOOL_INTERPOLATOR_VALUE {
            *output = false;
            return false;
        }
        *output = self.base.cached_value_0c != 0;
        self.base.last_time_08 = time;
        true
    }
}

// IDA: N/A resolves +0x10 for 006E8230. The link check guarantees the class, so a dangling link is a host defect.
// Completion: 100%
pub fn update_bool_interpolator(
    world: &mut World,
    interpolator: Entity,
    time: f32,
    output: &mut bool,
) -> bool {
    let mut state = *world
        .get::<BoolInterpolator>(interpolator)
        .expect("a NiBoolInterpolator entity");
    let data = state
        .data_10
        .map(|data| world.get::<BoolKeyData>(data).expect("linked NiBoolData"));
    let result = state.update(data, time, output);
    *world
        .get_mut::<BoolInterpolator>(interpolator)
        .expect("a NiBoolInterpolator entity") = state;
    result
}

// IDA: N/A resolves +0x10 for 006E8000.
// Completion: 100%
pub fn collapse_bool_interpolator(world: &mut World, interpolator: Entity) {
    let mut state = *world
        .get::<BoolInterpolator>(interpolator)
        .expect("a NiBoolInterpolator entity");
    let data = state
        .data_10
        .map(|data| world.get::<BoolKeyData>(data).expect("linked NiBoolData"));
    state.collapse(data);
    *world
        .get_mut::<BoolInterpolator>(interpolator)
        .expect("a NiBoolInterpolator entity") = state;
}

// IDA: N/A resolves +0x10 for 006E7C20.
// Completion: 100%
pub fn update_bool_timeline_interpolator(
    world: &mut World,
    interpolator: Entity,
    time: f32,
    output: &mut bool,
) -> bool {
    let mut state = *world
        .get::<BoolTimelineInterpolator>(interpolator)
        .expect("a NiBoolTimelineInterpolator entity");
    let data = state
        .base
        .data_10
        .map(|data| world.get::<BoolKeyData>(data).expect("linked NiBoolData"));
    let result = state.update_timeline(data, time, output);
    *world
        .get_mut::<BoolTimelineInterpolator>(interpolator)
        .expect("a NiBoolTimelineInterpolator entity") = state;
    result
}

// IDA: N/A resolves +0x10 for the inherited 006E8000; the timeline's own fields stay.
// Completion: 100%
pub fn collapse_bool_timeline_interpolator(world: &mut World, interpolator: Entity) {
    let mut state = *world
        .get::<BoolTimelineInterpolator>(interpolator)
        .expect("a NiBoolTimelineInterpolator entity");
    let data = state
        .base
        .data_10
        .map(|data| world.get::<BoolKeyData>(data).expect("linked NiBoolData"));
    state.base.collapse(data);
    *world
        .get_mut::<BoolTimelineInterpolator>(interpolator)
        .expect("a NiBoolTimelineInterpolator entity") = state;
}

#[cfg(test)]
mod tests {
    use super::super::registration::{FactoryCreate, NiFactoryRegistry};
    use super::super::stream_objects::NiClass;
    use super::*;

    fn keys(pairs: &[(f32, u8)]) -> Vec<BoolKey> {
        pairs
            .iter()
            .map(|&(time, value)| BoolKey { time, value })
            .collect()
    }

    fn step(world: &mut World, pairs: &[(f32, u8)]) -> Entity {
        world
            .spawn(BoolKeyData::new(BoolKeyType::STEP, keys(pairs)).unwrap())
            .id()
    }

    fn state(world: &World, interpolator: Entity) -> BoolInterpolator {
        *world.get::<BoolInterpolator>(interpolator).unwrap()
    }

    fn set(world: &mut World, interpolator: Entity, data: Entity) {
        world
            .get_mut::<BoolInterpolator>(interpolator)
            .unwrap()
            .set_data(Some(data));
    }

    /// Ported from `bool_animation_tests`: IDA 006E8370 / 006E8740 and 006E8230's invalid-output branches.
    #[test]
    fn cold_and_update() {
        let cold_data = BoolKeyData::default();
        assert_eq!(cold_data.key_type(), BoolKeyType::NONE);
        assert!(cold_data.keys().is_empty());
        let mut world = World::new();
        let interpolator = world.spawn(BoolInterpolator::default()).id();
        let cold = state(&world, interpolator);
        assert_eq!(cold.last_time_08, INVALID_BOOL_INTERPOLATOR_TIME);
        assert_eq!(cold.cached_value_0c, INVALID_BOOL_INTERPOLATOR_VALUE);
        assert_eq!(cold.last_key_index, 0);
        assert!(cold.data_10.is_none());
        let mut output = true;
        assert!(!update_bool_interpolator(
            &mut world,
            interpolator,
            0.0,
            &mut output
        ));
        assert!(!output);
        assert_eq!(
            state(&world, interpolator).last_time_08,
            INVALID_BOOL_INTERPOLATOR_TIME
        );
        output = true;
        assert!(!update_bool_interpolator(
            &mut world,
            interpolator,
            INVALID_BOOL_INTERPOLATOR_TIME,
            &mut output
        ));
        assert!(!output);
        assert_eq!(
            state(&world, interpolator).last_time_08,
            INVALID_BOOL_INTERPOLATOR_TIME
        );

        let data = step(&mut world, &[(0.0, 0), (1.0, 7)]);
        set(&mut world, interpolator, data);
        assert!(update_bool_interpolator(
            &mut world,
            interpolator,
            0.5,
            &mut output
        ));
        assert!(!output);
        assert_eq!(state(&world, interpolator).last_time_08, 0.5);
        output = false;
        assert!(update_bool_interpolator(
            &mut world,
            interpolator,
            1.0,
            &mut output
        ));
        assert!(output);
        assert_eq!(state(&world, interpolator).cached_value_0c, 1);
        assert_eq!(state(&world, interpolator).last_time_08, 1.0);
        output = false;
        assert!(update_bool_interpolator(
            &mut world,
            interpolator,
            1.0,
            &mut output
        ));
        assert!(output);
        output = true;
        assert!(!update_bool_interpolator(
            &mut world,
            interpolator,
            2.0,
            &mut output
        ));
        assert!(!output);
        assert_eq!(state(&world, interpolator).last_time_08, 1.0);
    }

    /// IDA: 006BDBA0 cached scan and 006C1C60 type-5 step output.
    #[test]
    fn sampling() {
        let data =
            BoolKeyData::new(BoolKeyType::STEP, keys(&[(0.0, 2), (1.0, 7), (2.0, 0)])).unwrap();
        let mut index = 0;
        assert_eq!(data.sample(1.5, &mut index), Some(7));
        assert_eq!(index, 1);
        assert_eq!(data.sample(0.5, &mut index), Some(2));
        assert_eq!(index, 0);
        assert_eq!(data.sample(1.0, &mut index), Some(7));
        assert_eq!(
            data.sample(INVALID_BOOL_INTERPOLATOR_TIME, &mut index),
            Some(2)
        );
        let linear = BoolKeyData::new(BoolKeyType::LINEAR, keys(&[(0.0, 1), (1.0, 1)])).unwrap();
        assert_eq!(linear.sample(0.5, &mut index), None);
    }

    /// IDA: 006E8000. The C++ `weak_ptr::expired` checks have no entity equivalent.
    #[test]
    fn collapse() {
        let mut world = World::new();
        let interpolator = world.spawn(BoolInterpolator::default()).id();
        let empty = world.spawn(BoolKeyData::default()).id();
        set(&mut world, interpolator, empty);
        collapse_bool_interpolator(&mut world, interpolator);
        assert!(state(&world, interpolator).data_10.is_none());
        assert_eq!(
            state(&world, interpolator).cached_value_0c,
            INVALID_BOOL_INTERPOLATOR_VALUE
        );

        let single = step(&mut world, &[(0.0, 7)]);
        set(&mut world, interpolator, single);
        let mut output = false;
        assert!(update_bool_interpolator(
            &mut world,
            interpolator,
            1.0,
            &mut output
        ));
        assert!(output);
        let previous = state(&world, interpolator);
        collapse_bool_interpolator(&mut world, interpolator);
        let collapsed = state(&world, interpolator);
        assert!(collapsed.data_10.is_none());
        assert_eq!(collapsed.cached_value_0c, 7);
        assert_eq!(collapsed.last_time_08, previous.last_time_08);
        assert_eq!(collapsed.last_key_index, previous.last_key_index);
        assert!(update_bool_interpolator(
            &mut world,
            interpolator,
            2.0,
            &mut output
        ));
        assert!(output);

        let equal = world
            .spawn(BoolKeyData::new(BoolKeyType::LINEAR, keys(&[(0.0, 7), (1.0, 7)])).unwrap())
            .id();
        set(&mut world, interpolator, equal);
        collapse_bool_interpolator(&mut world, interpolator);
        assert!(state(&world, interpolator).data_10.is_none());
        assert_eq!(state(&world, interpolator).cached_value_0c, 7);
        let different = step(&mut world, &[(0.0, 1), (1.0, 7)]);
        set(&mut world, interpolator, different);
        collapse_bool_interpolator(&mut world, interpolator);
        assert!(state(&world, interpolator).data_10.is_some());
    }

    /// IDA: 006E8740's shared NiBoolData / NiVisData address, 006E8370 and 006E7E70.
    #[test]
    fn factory_binding() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiBoolData", 0x006E_8740);
        registry.insert_or_assign("NiVisData", 0x006E_8740);
        registry.insert_or_assign("NiBoolInterpolator", 0x006E_8370);
        registry.insert_or_assign("NiBoolTimelineInterpolator", 0x006E_7E70);
        let created = |world: &mut World, registry: &NiFactoryRegistry, name: &str| match registry
            .create(world, name)
        {
            FactoryCreate::Created {
                requested_name,
                original_constructor,
                entity,
            } => (requested_name, original_constructor, entity),
            other => panic!("{name}: {other:?}"),
        };
        let (first_name, first_constructor, first) = created(&mut world, &registry, "NiBoolData");
        let (alias_name, alias_constructor, alias) = created(&mut world, &registry, "NiVisData");
        let (_, _, interpolator) = created(&mut world, &registry, "NiBoolInterpolator");
        let (_, timeline_constructor, timeline) =
            created(&mut world, &registry, "NiBoolTimelineInterpolator");
        assert_eq!(first_name, "NiBoolData");
        assert_eq!(alias_name, "NiVisData");
        assert_eq!(first_constructor, alias_constructor);
        assert_eq!(world.get::<NiClass>(first), Some(&NiClass::BoolKeyData));
        assert_eq!(world.get::<NiClass>(alias), Some(&NiClass::BoolKeyData));
        assert_ne!(first, alias);
        assert_eq!(state(&world, interpolator).cached_value_0c, 2);
        assert_eq!(timeline_constructor, 0x006E_7E70);
        assert_eq!(
            world
                .get::<BoolTimelineInterpolator>(timeline)
                .unwrap()
                .previous_key_index,
            0
        );
        registry.release_storage();
        assert!(world.get::<BoolKeyData>(alias).unwrap().keys().is_empty());
    }

    struct Timeline {
        world: World,
        entity: Entity,
    }

    impl Timeline {
        fn new() -> Self {
            let mut world = World::new();
            let entity = world.spawn(BoolTimelineInterpolator::default()).id();
            Self { world, entity }
        }
        fn set(&mut self, pairs: &[(f32, u8)]) {
            let data = step(&mut self.world, pairs);
            self.world
                .get_mut::<BoolTimelineInterpolator>(self.entity)
                .unwrap()
                .set_timeline_data(Some(data));
        }
        fn update(&mut self, time: f32, output: &mut bool) -> bool {
            update_bool_timeline_interpolator(&mut self.world, self.entity, time, output)
        }
        fn state(&self) -> BoolTimelineInterpolator {
            *self
                .world
                .get::<BoolTimelineInterpolator>(self.entity)
                .unwrap()
        }
    }

    /// IDA: 006E7E70 cold extra fields and 006E7C20's skipped-key update.
    #[test]
    fn timeline() {
        let mut timeline = Timeline::new();
        let cold = timeline.state();
        assert_eq!(cold.previous_key_index, 0);
        assert_eq!(cold.previous_raw_value, 0);
        assert_eq!(cold.base.cached_value_0c, INVALID_BOOL_INTERPOLATOR_VALUE);
        let mut output = true;
        assert!(!timeline.update(0.0, &mut output));
        assert!(!output);
        assert_eq!(
            timeline.state().base.last_time_08,
            INVALID_BOOL_INTERPOLATOR_TIME
        );
        output = true;
        assert!(!timeline.update(INVALID_BOOL_INTERPOLATOR_TIME, &mut output));
        assert!(!output);

        timeline.set(&[(0.0, 0), (1.0, 1), (2.0, 0), (3.0, 0)]);
        assert!(timeline.update(0.0, &mut output));
        assert!(!output);
        assert!(timeline.update(2.5, &mut output));
        assert!(output, "a skipped opposite key emits one pulse");
        assert_eq!(timeline.state().previous_key_index, 0);
        assert_eq!(timeline.state().base.last_key_index, 2);
        assert_eq!(timeline.state().previous_raw_value, 0);
        output = false;
        assert!(timeline.update(2.5, &mut output));
        assert!(output, "a repeated time returns the emitted pulse");
        assert!(timeline.update(2.75, &mut output));
        assert!(!output, "no skipped key restores the sampled value");

        timeline.set(&[(0.0, 0), (1.0, 0), (2.0, 0), (3.0, 0)]);
        assert!(timeline.update(0.0, &mut output));
        assert!(!output);
        assert!(timeline.update(2.5, &mut output));
        assert!(!output);

        timeline.set(&[
            (0.0, 0),
            (1.0, 0),
            (2.0, 0),
            (3.0, 0),
            (4.0, 1),
            (5.0, 0),
            (6.0, 0),
        ]);
        assert!(timeline.update(3.5, &mut output));
        assert!(!output);
        assert!(timeline.update(0.5, &mut output));
        assert!(output, "backward wrap tail scan");
        assert_eq!(timeline.state().previous_key_index, 3);
        assert_eq!(timeline.state().base.last_key_index, 0);

        timeline.set(&[(0.0, 0), (1.0, 1), (2.0, 0), (3.0, 0), (4.0, 0), (5.0, 0)]);
        assert!(timeline.update(3.5, &mut output));
        assert!(output);
        assert!(timeline.update(2.5, &mut output));
        assert!(output, "backward wrap head scan");
        assert_eq!(timeline.state().previous_key_index, 3);
        assert_eq!(timeline.state().base.last_key_index, 2);

        timeline.set(&[(0.0, 0), (1.0, 1), (2.0, 0), (3.0, 0)]);
        assert!(timeline.update(0.0, &mut output));
        assert!(!output);
        assert!(timeline.update(1.0, &mut output));
        assert!(output);
        assert_eq!(timeline.state().base.last_key_index, 0);
        assert!(timeline.update(2.0, &mut output));
        assert!(!output);
        assert_eq!(timeline.state().base.last_key_index, 1);
        output = true;
        assert!(timeline.update(2.0, &mut output));
        assert!(!output);

        timeline.set(&[(0.0, 1), (1.0, 1), (2.0, 1), (3.0, 1)]);
        assert!(timeline.update(2.5, &mut output));
        assert!(output);
        assert!(timeline.update(1.5, &mut output));
        assert!(output);
        let previous = timeline.state();
        collapse_bool_timeline_interpolator(&mut timeline.world, timeline.entity);
        let collapsed = timeline.state();
        assert!(collapsed.base.data_10.is_none());
        assert_eq!(previous.previous_key_index, 2);
        assert_eq!(previous.previous_raw_value, 1);
        assert_eq!(collapsed.previous_key_index, previous.previous_key_index);
        assert_eq!(collapsed.previous_raw_value, previous.previous_raw_value);
    }

    /// IDA: N/A host validated-input boundary for 006BDBA0.
    #[test]
    fn input_rejection() {
        assert!(BoolKeyData::new(BoolKeyType(2), keys(&[(0.0, 1)])).is_err());
        assert!(BoolKeyData::new(BoolKeyType::STEP, keys(&[(1.0, 1), (0.0, 0)])).is_err());
    }
}
