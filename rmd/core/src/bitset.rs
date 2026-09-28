use std::marker::PhantomData;

pub trait BitIndex: Copy {
    fn bit_index(self) -> usize;
}

/// one bit per id, for ids that count up densely
#[derive(Debug)]
pub struct BitSet<T> {
    words: Vec<u64>,
    marker: PhantomData<T>,
}

impl<T> Default for BitSet<T> {
    fn default() -> Self {
        Self {
            words: Vec::new(),
            marker: PhantomData,
        }
    }
}

impl<T: BitIndex> BitSet<T> {
    const BITS: usize = u64::BITS as usize;

    fn bit(id: T) -> (usize, u64) {
        let index = id.bit_index();

        (index / Self::BITS, 1 << (index % Self::BITS))
    }

    pub fn contains(&self, id: T) -> bool {
        let (word, mask) = Self::bit(id);

        self.words.get(word).is_some_and(|bits| bits & mask != 0)
    }

    /// false when the id was already in the set
    pub fn insert(&mut self, id: T) -> bool {
        let (word, mask) = Self::bit(id);
        if self.words.len() <= word {
            self.words.resize(word + 1, 0);
        }

        let bits = &mut self.words[word];
        let added = *bits & mask == 0;
        *bits |= mask;

        added
    }

    pub fn remove(&mut self, id: T) {
        let (word, mask) = Self::bit(id);
        if let Some(bits) = self.words.get_mut(word) {
            *bits &= !mask;
        }
    }

    pub fn is_empty(&self) -> bool { self.words.iter().all(|bits| *bits == 0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl BitIndex for usize {
        fn bit_index(self) -> usize { self }
    }

    #[test]
    fn ids_across_words_are_added_found_and_removed() {
        let mut set = BitSet::<usize>::default();
        assert!(set.is_empty());

        assert!(set.insert(3));
        assert!(set.insert(130));
        assert!(!set.insert(130));
        assert!(set.contains(3) && set.contains(130));
        assert!(!set.contains(2) && !set.contains(1000));

        set.remove(3);
        set.remove(130);
        set.remove(1000);
        assert!(set.is_empty());
    }
}
