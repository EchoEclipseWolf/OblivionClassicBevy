//! The original's BSSimpleList: an embedded head node and singly linked tail, where an empty head
//! means nothing was ever stored or the last item was removed.
//!
//! IDA: the node semantics of 0067B1E0, 00446CB0 and 0065C620. Nodes are kept in list order rather
//! than heap-linked, so a Rust index stands in for a node pointer. Completion: 100% from the head.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerList<T> {
    /// Slot 0 is the embedded head; `None` is a null item.
    nodes: Vec<Option<T>>,
}

impl<T> Default for PointerList<T> {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self { nodes: vec![None] }
    }
}

impl<T: Copy + PartialEq> PointerList<T> {
    /// IDA: 00446CB0 PointerList_PrependNonNull: an empty head takes the item; otherwise a new node
    /// takes the head's item and the head takes this one.
    pub fn prepend_non_null(&mut self, item: T) {
        if self.nodes[0].is_none() {
            self.nodes[0] = Some(item);
        } else {
            self.nodes.insert(0, Some(item));
        }
    }

    /// IDA: 0067B1E0 PointerList_AppendNonNull: an empty last node takes the item; otherwise a new
    /// node is appended.
    pub fn append_non_null(&mut self, item: T) {
        let last = self.nodes.last_mut().expect("the head always exists");
        if last.is_none() {
            *last = Some(item);
        } else {
            self.nodes.push(Some(item));
        }
    }

    /// IDA: 0065C620 PointerList_RemoveFirstMatchingItem from the head: a match at the head pulls the
    /// next node in or clears the item; a later match is unlinked.
    pub fn remove_first_matching(&mut self, item: T) {
        if let Some(index) = self.nodes.iter().position(|node| *node == Some(item)) {
            if self.nodes.len() == 1 {
                self.nodes[0] = None;
            } else {
                self.nodes.remove(index);
            }
        }
    }

    /// IDA: N/A a walk's swaps of node items, as 0052F6D0 and 0044D610 make them: the nodes stay, and `items` go in them in order. The list keeps its length.
    pub fn set_items(&mut self, items: &[T]) {
        let mut items = items.iter();
        for node in &mut self.nodes {
            if node.is_some() {
                *node = items.next().copied();
            }
        }
    }

    /// IDA: N/A an accessor.
    /// The items in list order, without null nodes.
    pub fn items(&self) -> impl Iterator<Item = T> + '_ {
        self.nodes.iter().filter_map(|node| *node)
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items().next().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_rules_match_the_original() {
        let mut list = PointerList::default();
        assert!(list.is_empty());
        list.prepend_non_null(1);
        assert_eq!(list.nodes.len(), 1, "an empty head takes the item");
        list.prepend_non_null(2);
        list.append_non_null(3);
        assert_eq!(list.items().collect::<Vec<_>>(), [2, 1, 3]);
        list.remove_first_matching(9);
        list.remove_first_matching(2);
        assert_eq!(
            list.items().collect::<Vec<_>>(),
            [1, 3],
            "the head pulls the next node in"
        );
        list.remove_first_matching(3);
        assert_eq!(list.items().collect::<Vec<_>>(), [1]);
        list.remove_first_matching(1);
        assert!(list.is_empty());
        assert_eq!(list.nodes.len(), 1, "the head stays, empty");
        list.append_non_null(4);
        assert_eq!(list.nodes.len(), 1, "an empty last node takes the item");
    }
}
