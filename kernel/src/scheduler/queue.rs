//! Static max-heap priority queue for the CA-PIP scheduler.
//!
//! Stores task IDs ranked by their effective priority (supplied at insertion).
//! All operations are O(log N) where N ≤ MAX_TASKS (16) — effectively O(1).
//! No dynamic allocation; the heap array is embedded in the struct.

/// Entry stored in the heap: task ID paired with its effective priority.
#[derive(Clone, Copy)]
struct HeapEntry {
    task_id: u8,
    effective_priority: u8,
}

/// Fixed-capacity max-heap priority queue.
///
/// `N` is the maximum number of entries (set to `MAX_TASKS` at the call site).
/// The heap property is: `effective_priority[parent] >= effective_priority[child]`.
pub struct PriorityQueue<const N: usize> {
    heap: [Option<HeapEntry>; N],
    len: usize,
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
        self.heap[self.len] = Some(HeapEntry { task_id, effective_priority });
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

    fn priority_at(&self, idx: usize) -> u8 {
        self.heap[idx].map_or(0, |e| e.effective_priority)
    }

    fn sift_up(&mut self, mut idx: usize) {
        while idx > 0 {
            let parent = (idx - 1) / 2;
            if self.priority_at(idx) > self.priority_at(parent) {
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

            if left < self.len && self.priority_at(left) > self.priority_at(largest) {
                largest = left;
            }
            if right < self.len && self.priority_at(right) > self.priority_at(largest) {
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
