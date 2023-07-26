#![warn(missing_docs)]
#![crate_name = "stream_utils"]
#![forbid(unsafe_code)]

//! Extra Stream adaptors and functions.
//!
//! To Extend [`Stream`] with methods in this crate, import the [`StreamUtils`] trait:
//!
//! ```
//! use stream_utils::StreamUtils;
//! ```
//!
//! Now, new methods like [`copied_multi_stream`][`StreamUtils::copied_multi_stream`] are available
//! on all streams.
//!
//! ```
//! use futures_util::stream; // or futures::stream;
//! use stream_utils::{CopiedMultiStream, StreamUtils};
//!
//! let stream = stream::iter(0..3);
//! let streams: Vec<CopiedMultiStream<usize, _>> = stream.copied_multi_stream(4);
//! ```

use futures_util::Stream;

mod copied_multi_stream;

pub use crate::copied_multi_stream::*;

/// A [`Stream`] blanket implementation trait that provides extra adaptors.
///
/// [`Stream`]: crate::Stream
/// [futures]: https://docs.rs/futures
/// [futures-StreamExt]: https://docs.rs/futures/0.3/futures/stream/trait.StreamExt.html
pub trait StreamUtils: Stream {
    /// Copies values from the inner stream into multiple new streams. Polls from inner stream one
    /// value and waits till all new streams have pulled a copied value.
    /// Note that not pulling from all new streams in sequence will result in an endless loop
    /// polling a Pending state. Essentially blocking.
    ///
    /// When the underlying stream terminates, all new streams which have allready pulled the last value will be [`Pending`].
    /// When all new streams have pulled the last value, all streams will terminate.
    ///
    /// [`Pending`]: std::task::Poll#variant.Pending
    fn copied_multi_stream<I>(self, i: usize) -> Vec<CopiedMultiStream<I, Self>>
    where
        Self: Sized,
    {
        copied_multi_stream(self, i)
    }
}

impl<T: Stream> StreamUtils for T {}
