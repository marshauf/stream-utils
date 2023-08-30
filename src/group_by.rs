use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
    task::{Poll, Waker},
};

use futures_util::{Stream, StreamExt};

#[derive(Clone, Debug)]
struct Groups<K, S, F>
where
    S: Stream,
    F: Fn(&S::Item) -> K,
{
    inner: S,
    current_kv: Option<(K, S::Item)>,
    key: F,

    groups: Vec<(K, Option<Waker>)>,
    group_by: Option<Waker>,
}

#[derive(Clone)]
pub struct GroupBy<K, S, F>
where
    S: Stream,
    F: Fn(&S::Item) -> K,
{
    inner: Arc<Mutex<Groups<K, S, F>>>,
}

pub fn group_by<K, S, F>(stream: S, f: F) -> GroupBy<K, S, F>
where
    S: Stream,
    F: Fn(&S::Item) -> K,
{
    GroupBy {
        inner: Arc::new(Mutex::new(Groups {
            inner: stream,
            current_kv: None,
            key: f,
            groups: Vec::new(),
            group_by: None,
        })),
    }
}

impl<K, S, F> Stream for GroupBy<K, S, F>
where
    K: Clone + PartialEq + Debug,
    S: Stream + Unpin,
    S::Item: Debug,
    F: Fn(&S::Item) -> K,
{
    type Item = Group<K, S, F>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut inner = self.inner.lock().unwrap();
        if let Some((k, v)) = inner.current_kv.take() {
            // Current kv exists, find corresponding Group waker
            if let Some((_, waker)) = inner.groups.iter_mut().find(|(key, _)| k == *key) {
                if let Some(waker) = waker.take() {
                    waker.wake();
                }
                inner.current_kv = Some((k, v));
                inner.group_by = Some(cx.waker().clone());
                return Poll::Pending;
            }

            // key has no corresponding Group, return new group
            inner.current_kv = Some((k.clone(), v));
            inner.groups.push((k.clone(), None));
            return Poll::Ready(Some(Group {
                key: k,
                inner: self.inner.clone(),
            }));
        }

        // Current kv is empty, poll underlying stream
        match inner.inner.poll_next_unpin(cx) {
            Poll::Ready(Some(v)) => {
                // Get key for value
                let key = (inner.key)(&v);

                // Find group for key
                if let Some((_, waker)) = inner.groups.iter().find(|(k, _)| *k == key) {
                    // Wake up group, if it exists
                    if let Some(waker) = waker {
                        waker.clone().wake();
                    }
                    inner.current_kv = Some((key, v));
                    Poll::Pending
                } else {
                    // key has no corresponding Group, return new group
                    inner.current_kv = Some((key.clone(), v));
                    inner.groups.push((key.clone(), None));
                    Poll::Ready(Some(Group {
                        key,
                        inner: self.inner.clone(),
                    }))
                }
            }
            Poll::Ready(None) => {
                inner.group_by.take();
                for group in inner.groups.iter_mut() {
                    if let Some(waker) = group.1.take() {
                        waker.wake();
                    }
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Group<K, S, F>
where
    S: Stream,
    S::Item: Debug,
    F: Fn(&S::Item) -> K,
{
    key: K,
    inner: Arc<Mutex<Groups<K, S, F>>>,
}

impl<K, S, F> Stream for Group<K, S, F>
where
    S: Stream + Unpin,
    K: PartialEq + Debug,
    S::Item: Debug,
    F: Fn(&S::Item) -> K,
{
    type Item = S::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut state = self.inner.lock().unwrap();

        // Is a kv already waiting?
        if let Some((k, v)) = state.current_kv.take() {
            if self.key == k {
                // current kv matches Group key
                Poll::Ready(Some(v))
            } else {
                // current kv doesn't match Group key
                // find waker for this key and wake it up
                if let Some(Some(waker)) = state
                    .groups
                    .iter_mut()
                    .find(|(key, _)| *key == k)
                    .map(|(_, waker)| waker)
                {
                    waker.clone().wake();
                };

                // Store context waker in state
                if let Some(waker) = state
                    .groups
                    .iter_mut()
                    .find(|(key, _)| *key == self.key)
                    .map(|(_, waker)| waker)
                {
                    *waker = Some(cx.waker().clone());
                };
                // Restore kv into state
                state.current_kv = Some((k, v));

                Poll::Pending
            }
        } else {
            // state is empty, poll underlying stream
            match state.inner.poll_next_unpin(cx) {
                Poll::Ready(Some(v)) => {
                    // Get key for value
                    let key = (state.key)(&v);
                    if self.key == key {
                        Poll::Ready(Some(v))
                    } else {
                        //  Store context waker in state
                        if let Some(waker) = state
                            .groups
                            .iter_mut()
                            .find(|(k, _)| *k == self.key)
                            .map(|(_, waker)| waker)
                        {
                            *waker = Some(cx.waker().clone());
                        };

                        // Store kv and wake up
                        if let Some(Some(waker)) = state
                            .groups
                            .iter_mut()
                            .find(|(k, _)| *k == key)
                            .map(|(_, waker)| waker)
                        {
                            waker.wake_by_ref();
                        }

                        if let Some(waker) = state.group_by.take() {
                            waker.wake();
                        }

                        state.current_kv = Some((key, v));
                        Poll::Pending
                    }
                }
                Poll::Ready(None) => {
                    // underlying stream is done,
                    // terminate this stream and wake up GroupBy
                    let waker = state
                        .groups
                        .iter_mut()
                        .find(|(key, _)| *key == self.key)
                        .map(|(_, waker)| waker);
                    if let Some(waker) = waker {
                        *waker = None
                    };
                    if let Some(waker) = state.group_by.take() {
                        waker.wake();
                    }

                    Poll::Ready(None)
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use futures_util::stream;
    use ntest_timeout::timeout;
    use tokio::task;

    use crate::StreamUtils;

    use super::*;

    #[tokio::test]
    #[timeout(200)]
    async fn test_async_random_collect() {
        let stream = stream::iter(0..10);
        let mut groups = stream.group_by(|k| k % 2 == 0);

        let mut tasks = task::JoinSet::new();
        while let Some(group) = groups.next().await {
            tasks.spawn(async move {
                let res: Vec<usize> = group.collect().await;
                assert_eq!(res.len(), 5);
            });
        }
        while let Some(res) = tasks.join_next().await {
            assert!(res.is_ok());
        }
    }

    #[tokio::test]
    #[timeout(200)]
    async fn test_ordered_collect() {
        let stream = stream::iter("Streaming".chars());
        let mut groups_iter = stream.group_by(|k| k.is_uppercase());

        let mut uppercase = groups_iter.next().await.unwrap();
        assert_eq!(Some('S'), uppercase.next().await);
        let lowercase = groups_iter.next().await.unwrap();
        assert_eq!("treaming", lowercase.collect::<String>().await);
        assert_eq!(None, uppercase.next().await);
    }

    #[tokio::test]
    #[timeout(200)]
    async fn test_different_stream_lengths() {
        let stream = stream::iter("abbcbcbcbba".chars());
        let mut groups_iter = stream.group_by(|k| *k);

        let mut tasks = Vec::new();
        while let Some(group) = groups_iter.next().await {
            tasks.push(task::spawn(async move { group.collect::<String>().await }));
        }
        assert_eq!("ccc", tasks.pop().unwrap().await.unwrap());
        assert_eq!("bbbbbb", tasks.pop().unwrap().await.unwrap());
        assert_eq!("aa", tasks.pop().unwrap().await.unwrap());
    }
}
