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
//! use stream_utils::StreamUtils;
//!
//! let stream = stream::iter(0..3);
//! let streams = stream.copied_multi_stream(4);
//! ```

use futures_util::Stream;
use group_by::{group_by, GroupBy};

mod copied_multi_stream;
mod group_by;

pub use crate::copied_multi_stream::*;

/// A [`Stream`] blanket implementation trait that provides extra adaptors.
///
/// [`Stream`]: crate::Stream
/// [futures]: https://docs.rs/futures
/// [futures-StreamExt]: https://docs.rs/futures/0.3/futures/stream/trait.StreamExt.html
pub trait StreamUtils: Stream {
    /// Copies values from the inner stream into multiple new streams. Polls from inner stream one
    /// value and waits till all new streams have pulled a copied value.
    /// Note that the internal buffer only buffers one value from the inner stream.
    /// Not pulling from all new streams in sequence will result in an endless loop
    /// polling a [`Pending`] state. Essentially blocking.
    ///
    /// When the underlying stream terminates, all new streams which have allready pulled the last value will be [`Pending`].
    /// When all new streams have pulled the last value, all streams will terminate.
    ///
    /// [`Pending`]: std::task::Poll#variant.Pending
    #[inline(always)]
    fn copied_multi_stream(self, i: usize) -> Vec<CopiedMultiStream<Self>>
    where
        Self: Sized,
    {
        copied_multi_stream(self, i)
    }

    /// Groups values based on a key returned by [`F`] and returns a  [`Group`] for each new key.
    /// Values from [`Self`] are forwarded to the corresponding [`Group`].
    ///
    /// One value at a time is forwarded. All Groups wait on one Group to pull the current value.
    /// Requires pulling from all Groups till the underlying stream is terminated.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures_util::{stream, StreamExt};
    /// use stream_utils::StreamUtils;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let stream = stream::iter(0..2);
    ///     let mut groups_iter = stream.group_by(|k| k % 2 == 0);
    ///
    ///     let mut even = groups_iter.next().await.unwrap();
    ///     assert_eq!(Some(0), even.next().await);
    ///     let mut uneven = groups_iter.next().await.unwrap();
    ///     assert_eq!(Some(1), uneven.next().await);
    /// }
    /// ```
    ///
    /// ```
    /// use futures_util::{stream, StreamExt};
    /// use tokio::task;
    /// use stream_utils::StreamUtils;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let stream = stream::iter("abbcbcbcbba".chars());
    ///     let mut groups_iter = stream.group_by(|k| *k);
    ///
    ///     let mut tasks = Vec::new();
    ///     while let Some(group) = groups_iter.next().await {
    ///         tasks.push(task::spawn(async move { group.collect::<String>().await }));
    ///     }
    ///     assert_eq!("ccc", tasks.pop().unwrap().await.unwrap());
    ///     assert_eq!("bbbbbb", tasks.pop().unwrap().await.unwrap());
    ///     assert_eq!("aa", tasks.pop().unwrap().await.unwrap());
    /// }
    /// ```
    #[inline(always)]
    fn group_by<K, F>(self, f: F) -> GroupBy<K, Self, F>
    where
        Self: Sized,
        F: Fn(&Self::Item) -> K,
    {
        group_by(self, f)
    }
}

impl<T: Stream> StreamUtils for T {}
