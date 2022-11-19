use std::collections::VecDeque;

pub struct FixedVecDeque<T> {
    inner: VecDeque<T>,
    max_len: usize,
}

impl<T> FixedVecDeque<T> {
    pub fn with_max_len(max_len: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(max_len),
            max_len,
        }
    }

    /// Appends an element to the back of the deque.
    /// Pops the front element if the maximum length was reached, returning it as Some(T).
    pub fn push_back(&mut self, value: T) -> Option<T> {
        let popped = if self.inner.len() == self.max_len {
            self.inner.pop_front()
        } else {
            None
        };
        self.inner.push_back(value);
        popped
    }
}

impl<T> std::ops::Deref for FixedVecDeque<T> {
    type Target = VecDeque<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> std::ops::DerefMut for FixedVecDeque<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
