//! Original sorts. Their order is observable (equal keys keep the order the passes leave them in), so `sort_by` can't
//! stand in.

// IDA: 00506F00 ShellSortPointerArray: the gap goes 3h + 1 from 1 while it is at most (n - 1) / 9 (signed), then down by
// thirds; each pass an insertion sort by that gap, stopping at a compare of 0 or more. Not a stable sort. A compare's
// error stops the sort where it is (the port's stops); an infallible compare takes `E = Infallible`. Completion: 100%.
pub fn shell_sort<T: Clone, E>(
    items: &mut [T],
    mut compare: impl FnMut(&T, &T) -> Result<i32, E>,
) -> Result<(), E> {
    let count = items.len() as i32;
    let mut gap = 1_i32;
    if (count - 1) / 9 >= 1 {
        while gap <= (count - 1) / 9 {
            gap = 3 * gap + 1;
        }
    }
    while gap > 0 {
        for start in gap..count {
            let held = items[start as usize].clone();
            let mut at = start;
            while at >= gap {
                if compare(&held, &items[(at - gap) as usize])? >= 0 {
                    break;
                }
                items[at as usize] = items[(at - gap) as usize].clone();
                at -= gap;
            }
            items[at as usize] = held;
        }
        gap /= 3;
    }
    Ok(())
}
