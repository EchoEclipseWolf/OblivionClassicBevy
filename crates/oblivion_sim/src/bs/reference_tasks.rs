//! The model loader's reference tasks: 00438060 ModelLoader_QueueReference's QueuedReference family (QueuedReference, QueuedCharacter, QueuedCreature, QueuedPlayer, QueuedTree), the loader's reference map (+0x08), and the two queues a finished task goes through: the clone thread's (0043C530 QueuedReference_QueueOnCloneThread) and the finish queue the main thread drains (0043A5F0, 00433590).
//! A reference task never enters the IO manager's walk itself: only its QueuedModel child does, and the child's finish notifies the parent (`QueuedFile_NotifyParent`). The TES side (`tes::reference_load`) runs the tasks' virtuals. Host substitution (D9): the clone thread and the finish queue run on the main thread, in queue order.

use std::collections::{BTreeMap, VecDeque};

use bevy_ecs::prelude::*;

use super::model_loader::{IoTaskId, ModelId};

/// IDA: N/A a reference task's identity, a host value for its pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReferenceTaskId(pub usize);

/// IDA: the class 00438060 picks by the base's type, by its vtable: QueuedReference (`00A36D5C`, the default), QueuedCharacter (`00A36DDC`, an NPC 0x23), QueuedCreature (0x24), QueuedPlayer (the player), QueuedTree (0x1E).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueuedReferenceClass {
    Reference,
    Character,
    Creature,
    Player,
    Tree,
    /// IDA: 00A36C4C QueuedMagicItem (0x28), held here as a parent of its model children: +0x20 the item.
    MagicItem,
}

/// IDA: 00437C30 QueuedReference_Constructor (0x38; the player's and the NPC's 0x40): 00436500 IOTask (+0x08 references, +0x0C state, +0x10 the key, the priority in its byte 2), +0x18 the parent and +0x1C the children (`QueuedChildren`: +0x0C the count, +0x10 the finished count), +0x20 the reference, +0x24 the model child, +0x28 the Model, +0x2C the clone, +0x30 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedReference {
    pub class: QueuedReferenceClass,
    pub references_08: u32,
    pub state_0c: u32,
    pub key_10: u64,
    pub children_1c: Vec<IoTaskId>,
    pub children_finished_1c_10: u32,
    pub reference_20: Entity,
    pub child_24: Option<IoTaskId>,
    pub model_28: Option<ModelId>,
    pub clone_2c: Option<Entity>,
}

impl QueuedReference {
    /// IDA: the key's byte 2, the priority the constructor stores.
    #[must_use]
    pub const fn priority(&self) -> u8 {
        (self.key_10 >> 16) as u8
    }

    /// IDA: 0043CAE0's test on +0x1C: no children list, or its count equal to its finished count.
    #[must_use]
    pub fn children_done(&self) -> bool {
        self.children_1c.len() as u32 == self.children_finished_1c_10
    }
}

/// IDA: N/A the loader's reference tasks: the store, the +0x08 map (reference to task), and the clone thread's and finish queues, in arrival order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReferenceTasks {
    pub store: Vec<Option<QueuedReference>>,
    /// +0x08's entries, by the reference.
    pub map_08: BTreeMap<Entity, ReferenceTaskId>,
    /// The clone thread's queue (`QueuedReference_QueueOnCloneThread`).
    pub clone_queue: VecDeque<ReferenceTaskId>,
    /// The finish queue (`0043A5F0`), which the main thread's 00433590 drains.
    pub finish_queue: VecDeque<ReferenceTaskId>,
}

impl ReferenceTasks {
    /// IDA: N/A the task of an ID, if not released.
    #[must_use]
    pub fn get(&self, id: ReferenceTaskId) -> Option<&QueuedReference> {
        self.store.get(id.0).and_then(Option::as_ref)
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn get_mut(&mut self, id: ReferenceTaskId) -> Option<&mut QueuedReference> {
        self.store.get_mut(id.0).and_then(Option::as_mut)
    }

    /// IDA: N/A the store's push, the task's FormHeap block.
    pub fn insert(&mut self, task: QueuedReference) -> ReferenceTaskId {
        self.store.push(Some(task));
        ReferenceTaskId(self.store.len() - 1)
    }
}
