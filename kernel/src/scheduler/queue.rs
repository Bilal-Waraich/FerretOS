//! Static max-heap priority queue for the CA-PIP scheduler.
//!
//! Stores task IDs ranked by their effective priority (supplied at insertion).
//! Entries of equal priority leave in insertion order (FIFO), which is what
//! makes the scheduler's quantum-expiry rotation genuinely round-robin: a
//! binary heap on priority alone breaks ties arbitrarily.
//! All operations are O(log N) where N ≤ MAX_TASKS (16) — effectively O(1).
//! No dynamic allocation; the heap array is embedded in the struct.

/// Entry stored in the heap: task ID paired with its effective priority.
#[derive(Clone, Copy)]
struct HeapEntry {
    task_id: u8,
    effective_priority: u8,
    /// Insertion sequence number; lower = inserted earlier.
    seq: u32,
}

impl HeapEntry {
    /// Heap ordering: higher priority first, then earlier insertion.
    ///
    /// `seq` comparison uses wrapping distance so ordering stays correct
    /// across `u32` overflow, provided fewer than 2^31 inserts separate two
    /// live entries (at most N ≤ 16 entries are ever live).
    fn outranks(&self, other: &HeapEntry) -> bool {
        if self.effective_priority != other.effective_priority {
            return self.effective_priority > other.effective_priority;
        }
        (other.seq.wrapping_sub(self.seq) as i32) > 0
    }
}

/// Fixed-capacity max-heap priority queue.
///
/// `N` is the maximum number of entries (set to `MAX_TASKS` at the call site).
/// The heap property is: `effective_priority[parent] >= effective_priority[child]`.
pub struct PriorityQueue<const N: usize> {
    heap: [Option<HeapEntry>; N],
    len: usize,
    next_seq: u32,
}

impl<const N: usize> Default for PriorityQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> PriorityQueue<N> {
    /// Construct an empty priority queue.
    pub const fn new() -> Self {
        PriorityQueue {
            heap: [const { None }; N],
            len: 0,
            next_seq: 0,
        }
    }

    /// Number of entries currently in the queue.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the queue contains no entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Insert `task_id` with the given `effective_priority`.
    ///
    /// # Panics
    ///
    /// Panics if the queue is already at capacity (`len == N`).
    pub fn insert(&mut self, task_id: u8, effective_priority: u8) {
        assert!(self.len < N, "PriorityQueue: capacity exceeded");
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.heap[self.len] = Some(HeapEntry { task_id, effective_priority, seq });
        self.len += 1;
        self.sift_up(self.len - 1);
    }

    /// Remove and return the task ID with the highest effective priority.
    ///
    /// Returns `None` if the queue is empty.
    pub fn pop_max(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let root = self.heap[0]?.task_id;
        self.len -= 1;
        self.heap[0] = self.heap[self.len];
        self.heap[self.len] = None;
        if self.len > 0 {
            self.sift_down(0);
        }
        Some(root)
    }

    /// Return the task ID with the highest effective priority without removing it.
    ///
    /// Returns `None` if the queue is empty.
    pub fn peek_max(&self) -> Option<u8> {
        self.heap[0].map(|e| e.task_id)
    }

    /// Return the effective priority of the max entry, or 0 if empty.
    pub fn peek_max_priority(&self) -> u8 {
        self.heap[0].map_or(0, |e| e.effective_priority)
    }

    // --- internal heap operations ---

    /// Whether the entry at `a` should sit above the entry at `b`.
    fn outranks_at(&self, a: usize, b: usize) -> bool {
        match (&self.heap[a], &self.heap[b]) {
            (Some(x), Some(y)) => x.outranks(y),
            (Some(_), None) => true,
            _ => false,
        }
    }

    /// Remove `task_id` from the queue if present.  Returns whether it was found.
    ///
    /// O(N) scan plus O(log N) repair.
    pub fn remove(&mut self, task_id: u8) -> bool {
        let Some(idx) = (0..self.len).find(|&i| self.heap[i].is_some_and(|e| e.task_id == task_id))
        else {
            return false;
        };
        self.len -= 1;
        self.heap[idx] = self.heap[self.len];
        self.heap[self.len] = None;
        if idx < self.len {
            self.sift_down(idx);
            self.sift_up(idx);
        }
        true
    }

    fn sift_up(&mut self, mut idx: usize) {
        while idx > 0 {
            let parent = (idx - 1) / 2;
            if self.outranks_at(idx, parent) {
                self.heap.swap(idx, parent);
                idx = parent;
            } else {
                break;
            }
        }
    }

    fn sift_down(&mut self, mut idx: usize) {
        loop {
            let left = 2 * idx + 1;
            let right = 2 * idx + 2;
            let mut largest = idx;

            if left < self.len && self.outranks_at(left, largest) {
                largest = left;
            }
            if right < self.len && self.outranks_at(right, largest) {
                largest = right;
            }
            if largest == idx {
                break;
            }
            self.heap.swap(idx, largest);
            idx = largest;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PriorityQueue;

    #[test]
    fn pops_highest_priority_first() {
        let mut q: PriorityQueue<8> = PriorityQueue::new();
        q.insert(0, 1);
        q.insert(1, 3);
        q.insert(2, 2);
        assert_eq!(q.pop_max(), Some(1));
        assert_eq!(q.pop_max(), Some(2));
        assert_eq!(q.pop_max(), Some(0));
        assert_eq!(q.pop_max(), None);
    }

    #[test]
    fn equal_priorities_leave_in_fifo_order() {
        let mut q: PriorityQueue<8> = PriorityQueue::new();
        for id in 0..6 {
            q.insert(id, 5);
        }
        for id in 0..6 {
            assert_eq!(q.pop_max(), Some(id));
        }
    }

    #[test]
    fn reinserted_task_goes_behind_equal_peers() {
        // Round-robin: the preempted task re-enters behind tasks of equal
        // priority that were already waiting.
        let mut q: PriorityQueue<8> = PriorityQueue::new();
        q.insert(1, 3);
        q.insert(2, 3);
        let running = q.pop_max().unwrap();
        assert_eq!(running, 1);
        q.insert(running, 3);
        assert_eq!(q.pop_max(), Some(2));
        assert_eq!(q.pop_max(), Some(1));
    }

    #[test]
    fn fifo_survives_sequence_wraparound() {
        let mut q: PriorityQueue<4> = PriorityQueue::new();
        q.next_seq = u32::MAX - 1;
        q.insert(7, 1);
        q.insert(8, 1);
        q.insert(9, 1); // seq wraps to 0
        assert_eq!(q.pop_max(), Some(7));
        assert_eq!(q.pop_max(), Some(8));
        assert_eq!(q.pop_max(), Some(9));
    }

    #[test]
    fn remove_keeps_heap_order() {
        let mut q: PriorityQueue<8> = PriorityQueue::new();
        q.insert(0, 4);
        q.insert(1, 2);
        q.insert(2, 6);
        q.insert(3, 1);
        assert!(q.remove(0));
        assert!(!q.remove(0));
        assert_eq!(q.pop_max(), Some(2));
        assert_eq!(q.pop_max(), Some(1));
        assert_eq!(q.pop_max(), Some(3));
    }
}
