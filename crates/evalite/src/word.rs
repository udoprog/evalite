//! One 64-bit word of machine memory.
//!
//! Memory is untyped: the verifier has proved what every word holds, and
//! this module is the only code that knows how each kind is laid out in
//! the bits:
//!
//! - an `Int` is its `i64`, two's complement;
//! - a `Bool` is 0 or 1;
//! - a reference packs where a string's, list's or record's parts start
//!   (the low 32 bits) and how many there are (the high 32: a string's
//!   bytes, a list's entries or a record's words), never all ones;
//! - an `Option<Int>` is two words, a presence word (a `Bool`: 1 present,
//!   0 absent) then the `Int`; every other option is one word,
//!   [`Word::NONE`] when absent;
//! - a string's bytes are packed 8 to a word, little-endian.
//!
//! Nothing outside this module sees the raw `u64`.

/// A word of memory, built and taken apart only as one of its kinds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub(crate) struct Word(u64);

impl Word {
    /// The word of an absent option, other than an `Option<Int>`: all
    /// ones, which no `Bool` (0 or 1) or reference (an address and a
    /// length that each fit in 32 bits) ever is.
    pub(crate) const NONE: Self = Self(u64::MAX);

    /// An `Int`.
    #[inline]
    pub(crate) const fn int(n: i64) -> Self {
        Self(n as u64)
    }

    /// A `Bool`: 1 for true, 0 for false.
    #[inline]
    pub(crate) const fn bool(b: bool) -> Self {
        Self(b as u64)
    }

    /// The presence word of an `Option<Int>`, which is a `Bool`.
    #[inline]
    pub(crate) const fn present(present: bool) -> Self {
        Self::bool(present)
    }

    /// A reference to the `len` parts from `at`: `at` in the low 32 bits,
    /// `len` in the high, refusing either past 32 bits, and the one pair
    /// (`u32::MAX` parts from `u32::MAX`) that would be [`Word::NONE`],
    /// which ends past any memory.
    #[inline]
    pub(crate) fn reference(at: usize, len: usize) -> Result<Self, WordFault> {
        let at = u32::try_from(at).map_err(|_| WordFault::WideAddress)?;
        let len = u32::try_from(len).map_err(|_| WordFault::WideLength)?;
        let word = Self(u64::from(at) | (u64::from(len) << 32));

        if word.is_none() {
            return Err(WordFault::NoneReference);
        }

        Ok(word)
    }

    /// Eight packed bytes of a string.
    #[inline]
    pub(crate) const fn bytes(bytes: [u8; 8]) -> Self {
        Self(u64::from_le_bytes(bytes))
    }

    /// The word as an `Int`.
    #[inline]
    pub(crate) const fn as_int(self) -> i64 {
        self.0 as i64
    }

    /// The word as a `Bool`, a fault on anything but 0 or 1.
    #[inline]
    pub(crate) fn as_bool(self) -> Result<bool, WordFault> {
        if self.0 > 1 {
            return Err(WordFault::NotABool);
        }

        Ok(self.0 == 1)
    }

    /// The word as a reference: where its parts start, and how many there
    /// are.
    #[inline]
    pub(crate) fn as_reference(self) -> Result<(usize, usize), WordFault> {
        let at = (self.0 & u64::from(u32::MAX)) as u32;
        let len = (self.0 >> 32) as u32;
        match (usize::try_from(at), usize::try_from(len)) {
            (Ok(at), Ok(len)) => Ok((at, len)),
            _ => Err(WordFault::PastAddressSpace),
        }
    }

    /// Whether the word is [`Word::NONE`], an absent option.
    #[inline]
    pub(crate) const fn is_none(self) -> bool {
        self.0 == u64::MAX
    }

    /// The eight packed bytes of a string the word holds.
    #[inline]
    pub(crate) const fn as_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }
}

/// Why a word does not decode, or a reference cannot be packed: one
/// byte, so the checks inlined into the run loop stay a compare and a
/// branch, and only a fault builds a [`RunError`](crate::RunError).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WordFault {
    WideAddress,
    WideLength,
    NoneReference,
    NotABool,
    PastAddressSpace,
}

impl WordFault {
    /// What the program did wrong, for its fault.
    pub(crate) const fn what(self) -> &'static str {
        match self {
            Self::WideAddress => "memory past what a word can address",
            Self::WideLength => "a value too long for a word",
            Self::NoneReference => "a reference past the end of memory",
            Self::NotABool => "a Bool other than 0 or 1",
            Self::PastAddressSpace => "a size past the address space",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Word, WordFault};

    #[test]
    fn ints_round_trip() {
        for n in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(Word::int(n).as_int(), n);
        }
    }

    #[test]
    fn bools_round_trip() {
        assert_eq!(Word::bool(true).as_bool().ok(), Some(true));
        assert_eq!(Word::bool(false).as_bool().ok(), Some(false));
        assert_eq!(Word::present(true), Word::bool(true));
        assert_eq!(Word::present(false), Word::bool(false));
    }

    #[test]
    fn bools_other_than_0_or_1_fault() {
        assert_eq!(Word::int(2).as_bool(), Err(WordFault::NotABool));
        assert_eq!(Word::int(-1).as_bool(), Err(WordFault::NotABool));
        assert_eq!(Word::NONE.as_bool(), Err(WordFault::NotABool));
    }

    #[test]
    fn references_round_trip_up_to_32_bits() {
        let max = u32::MAX as usize;

        for (at, len) in [(0, 0), (1, 2), (max, 0), (0, max), (max - 1, max)] {
            let word = Word::reference(at, len).unwrap();
            assert_eq!(word.as_reference().unwrap(), (at, len));
        }
    }

    #[test]
    fn references_past_32_bits_are_refused() {
        let past = u32::MAX as usize + 1;
        assert_eq!(Word::reference(past, 0), Err(WordFault::WideAddress));
        assert_eq!(Word::reference(0, past), Err(WordFault::WideLength));
        let max = u32::MAX as usize;
        assert_eq!(Word::reference(max, max), Err(WordFault::NoneReference));
    }

    #[test]
    fn none_is_no_bool_or_reference() {
        assert!(Word::NONE.is_none());
        assert_ne!(Word::NONE, Word::bool(true));
        assert_ne!(Word::NONE, Word::bool(false));

        let max = u32::MAX as usize;

        for (at, len) in [(0, 0), (max - 1, max), (max, max - 1)] {
            let word = Word::reference(at, len).unwrap();
            assert_ne!(word, Word::NONE);
            assert!(!word.is_none());
        }

        assert!(!Word::bool(true).is_none());
        assert!(!Word::int(0).is_none());
    }

    #[test]
    fn bytes_round_trip() {
        let bytes = *b"evalite!";
        assert_eq!(Word::bytes(bytes).as_bytes(), bytes);
    }
}
