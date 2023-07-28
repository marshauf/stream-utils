use std::{
    sync::{Arc, Mutex},
    task::Waker,
};

use futures_util::{Stream, StreamExt};

#[derive(Clone)]
struct CopiedMultiStreamState<S>
where
    S: Stream,
{
    cache: Box<[Option<S::Item>]>,
    wakers: Box<[Option<Waker>]>,
    stream: Option<S>,
}

/// Stream for the [`copied_multi_stream`](crate::StreamUtils::copied_multi_stream) method.
#[must_use = "streams do nothing unless polled"]
#[derive(Clone)]
pub struct CopiedMultiStream<S>
where
    S: Stream,
{
    state: Arc<Mutex<CopiedMultiStreamState<S>>>,
    pos: usize,
}

/// Copies values from the inner stream into multiple new streams. Polls from inner stream one
/// value and waits till all new streams have pulled a copied value.
/// Note that not pulling from all new streams will result in a blocking state.
///
/// When the underlying stream terminates, all new streams which have allready pulled the last value will be [`Pending`].
/// When all new streams have pulled the last value, all streams will terminate on next pull.
///
/// [`Pending`]: std::task::Poll#variant.Pending
pub fn copied_multi_stream<S>(stream: S, i: usize) -> Vec<CopiedMultiStream<S>>
where
    S: Stream,
{
    let state = Arc::new(Mutex::new(CopiedMultiStreamState {
        stream: Some(stream),
        cache: (0..i).map(|_| None).collect(),
        wakers: (0..i).map(|_| None).collect(),
    }));
    (0..i)
        .map(|pos| CopiedMultiStream {
            pos,
            state: state.clone(),
        })
        .collect()
}

impl<S> Stream for CopiedMultiStream<S>
where
    S: Stream + Unpin,
    S::Item: Clone,
{
    type Item = S::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut state = self.state.lock().unwrap();
        if let Some(v) = state.cache[self.pos].take() {
            std::task::Poll::Ready(Some(v))
        } else if state.cache.iter().any(Option::is_some) {
            state.wakers[self.pos] = Some(cx.waker().clone());
            std::task::Poll::Pending
        } else if let Some(ref mut stream) = state.stream {
            match stream.poll_next_unpin(cx) {
                std::task::Poll::Ready(Some(v)) => {
                    state.cache.iter_mut().for_each(|c| *c = Some(v.clone()));
                    state.wakers.iter_mut().for_each(|waker| {
                        if let Some(waker) = waker.take() {
                            waker.wake_by_ref()
                        }
                    });
                    std::task::Poll::Ready(state.cache[self.pos].take())
                }
                std::task::Poll::Ready(None) => {
                    state.stream = None;
                    state.wakers.iter_mut().for_each(|waker| {
                        if let Some(waker) = waker.take() {
                            waker.wake_by_ref()
                        }
                    });
                    std::task::Poll::Ready(None)
                }
                std::task::Poll::Pending => {
                    state.wakers[self.pos] = Some(cx.waker().clone());
                    std::task::Poll::Pending
                }
            }
        } else {
            std::task::Poll::Ready(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use futures_util::stream::{self, BoxStream};
    use ntest_timeout::timeout;

    use crate::StreamUtils;

    use super::*;

    #[tokio::test]
    async fn test_stream() {
        let size = 3;
        let stream = stream::iter(0..3);
        let res = stream.copied_multi_stream(size);

        assert_eq!(res.len(), size);
        let res = stream::select_all(res);
        let res: Vec<usize> = res.collect().await;
        assert_eq!(res, vec![0, 0, 0, 1, 1, 1, 2, 2, 2]);
    }

    #[tokio::test]
    async fn test_box_stream() {
        let size = 3;
        let stream: BoxStream<usize> = Box::pin(stream::iter(0..3));
        let res = stream.copied_multi_stream(size);
        assert_eq!(res.len(), size);
        let res = stream::select_all(res);
        let res: Vec<usize> = res.collect().await;
        assert_eq!(res, vec![0, 0, 0, 1, 1, 1, 2, 2, 2]);
    }

    #[tokio::test]
    async fn test_empty_stream() {
        let size = 3;
        let stream = Box::pin(stream::iter(0..0));
        let res = stream.copied_multi_stream(size);
        assert_eq!(res.len(), size);
        let res = stream::select_all(res);
        let res: Vec<usize> = res.collect().await;
        let exp: Vec<usize> = Vec::new();
        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn test_zero_streams() {
        let size = 0;
        let stream = stream::iter(0..3);
        let res = stream.copied_multi_stream(size);
        assert_eq!(res.len(), size);
        let res = stream::select_all(res);
        let res: Vec<usize> = res.collect().await;
        let exp: Vec<usize> = Vec::new();
        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn test_future_stream() {
        let size = 3;
        let stream = stream::unfold(0, |state| async move {
            if state <= 2 {
                let next_state = state + 1;
                let yielded = state * 2;
                Some((yielded, next_state))
            } else {
                None
            }
        });
        let stream = pin!(stream);
        let res = stream.copied_multi_stream(size);
        assert_eq!(res.len(), size);
        let res = stream::select_all(res);
        let res: Vec<usize> = res.collect().await;
        assert_eq!(res, vec![0, 0, 0, 2, 2, 2, 4, 4, 4]);
    }

    #[tokio::test]
    #[timeout(200)]
    async fn test_async_pull() {
        let size = 5;
        let stream = stream::iter(0..3);
        let res = stream.copied_multi_stream(size);

        let res: Vec<_> = res
            .into_iter()
            .map(|stream| tokio::task::spawn(async move { stream.collect::<Vec<usize>>().await }))
            .collect();
        for r in res {
            let r = r.await.unwrap();
            assert_eq!(r, vec![0, 1, 2]);
        }
    }
}
