//! Debug information: where in the source each instruction of a program
//! came from.

use musli::{Decode, Encode};
use syntree::Span;

use crate::program::StoreError;

/// Where in its source each instruction of a [`Program`](crate::Program)
/// came from, for a [`RunError`](crate::RunError)'s labels to point at the
/// expression that failed.
///
/// It is made beside the program by [`compile_debug`](crate::compile_debug)
/// and kept apart from it: a program runs without it, and a caller that
/// wants spans stores it next to the program.
///
/// ```
/// use evalite::{DebugInfo, Environment, Options};
///
/// let env = Environment::with_builtins();
/// let (_, debug) = evalite::compile_debug("1 + 2", &env, &Options::new())?;
///
/// let loaded = DebugInfo::from_bytes(&debug.to_bytes()?)?;
/// assert_eq!(loaded, debug);
///
/// assert!(DebugInfo::from_bytes(b"not debug info").is_err());
/// # Ok::<_, Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DebugInfo {
    /// The span of each instruction, by its index.
    spans: Vec<Span<u32>>,
}

#[derive(Encode, Decode)]
struct StoredDebug {
    spans: Vec<(u32, u32)>,
}

impl DebugInfo {
    pub(crate) fn new(spans: Vec<Span<u32>>) -> Self {
        Self { spans }
    }

    /// The span of instruction `pc`.
    pub(crate) fn span(&self, pc: u32) -> Option<Span<u32>> {
        self.spans.get(usize::try_from(pc).ok()?).copied()
    }

    /// Encode it with `musli`.
    ///
    /// # Errors
    ///
    /// When it cannot be encoded.
    pub fn to_bytes(&self) -> Result<Vec<u8>, StoreError> {
        let stored = StoredDebug {
            spans: self.spans.iter().map(|s| (s.start, s.end)).collect(),
        };

        musli::storage::to_vec(&stored).map_err(|e| StoreError::Encode(e.to_string()))
    }

    /// Decode what [`to_bytes`](Self::to_bytes) gave.
    ///
    /// # Errors
    ///
    /// When `bytes` are not stored debug info, or a span ends before it
    /// starts.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        let stored: StoredDebug =
            musli::storage::from_slice(bytes).map_err(|e| StoreError::Decode(e.to_string()))?;

        let spans = stored
            .spans
            .into_iter()
            .map(|(start, end)| {
                if start > end {
                    return Err(StoreError::Invalid(format!(
                        "a span from {start} ends at {end}"
                    )));
                }

                Ok(Span::new(start, end))
            })
            .collect::<Result<_, _>>()?;

        Ok(Self { spans })
    }
}
