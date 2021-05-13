// SPDX-License-Identifier: GPL-3.0-or-later
use futures::task::{AtomicWaker, Context, Poll};
use futures::Future;

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug)]
pub struct InnerTreeCount {
    parent: Option<TreeCount>,
    count: AtomicUsize,
    waker: AtomicWaker,
}

#[derive(Clone, Debug)]
pub struct TreeCount(Arc<InnerTreeCount>);

#[derive(Debug)]
pub struct CountToken {
    node: Arc<InnerTreeCount>,
}

impl InnerTreeCount {
    fn add(&self, val: usize) {
        if let Some(parent) = &self.parent {
            parent.0.add(val);
        }
        let old_size = self.count.fetch_add(val, Ordering::AcqRel);
        if old_size == usize::MAX {
            panic!("Tree count has overflowed");
        }
        self.waker.wake();
    }

    fn remove(&self, val: usize) {
        if let Some(parent) = &self.parent {
            parent.0.remove(val);
        }
        let old_size = self.count.fetch_sub(val, Ordering::AcqRel);
        if old_size == usize::MIN {
            panic!("Tree count has underflowed");
        }
    }
}

impl TreeCount {
    pub fn new() -> Self {
        Self(Arc::new(InnerTreeCount {
            parent: None,
            count: AtomicUsize::new(0),
            waker: AtomicWaker::new(),
        }))
    }

    pub fn new_child(&self) -> Self {
        let parent = self.clone();
        Self(Arc::new(InnerTreeCount {
            parent: Some(parent),
            count: AtomicUsize::new(0),
            waker: AtomicWaker::new(),
        }))
    }

    pub fn get_token(&self) -> CountToken {
        CountToken::new(self)
    }

    pub fn count(&self) -> usize {
        self.0.count.load(Ordering::Acquire)
    }
}

impl Future for TreeCount {
    type Output = usize;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let count = self.count();
        if count > 0 {
            return Poll::Ready(count);
        }
        self.0.waker.register(cx.waker());
        match self.count() {
            0 => Poll::Pending,
            n => Poll::Ready(n),
        }
    }
}

impl CountToken {
    fn new(node: &TreeCount) -> Self {
        let node = Arc::clone(&node.0);
        node.add(1);
        Self { node }
    }
}

impl Clone for CountToken {
    fn clone(&self) -> Self {
        // When cloning a token, add one to the parent tree node to account for the eventual drop.
        let node = Arc::clone(&self.node);
        node.add(1);
        Self { node }
    }
}

impl Drop for CountToken {
    fn drop(&mut self) {
        self.node.remove(1);
    }
}

#[cfg(test)]
mod test {
    use super::TreeCount;
    use std::time::{Duration, Instant};
    use tokio::time::{sleep, timeout};

    #[test]
    fn single_node_count() {
        let root = TreeCount::new();
        assert_eq!(root.count(), 0, "Initial count of a root node is not 0");
        let token = root.get_token();
        assert_eq!(
            root.count(),
            1,
            "Root node count did not increment for a token"
        );
        drop(token);
        assert_eq!(
            root.count(),
            0,
            "Root node count did not decrement when token dropped"
        )
    }

    #[test]
    fn clone_token() {
        let root = TreeCount::new();
        let token1 = root.get_token();
        assert_eq!(
            root.count(),
            1,
            "Node count did not increase for token creation"
        );
        let token2 = token1.clone();
        assert_eq!(
            root.count(),
            2,
            "Node count did not increase when a token was cloned"
        );
        drop(token1);
        assert_eq!(
            root.count(),
            1,
            "Node count did not decrease by 1 when a cloned token source was dropped"
        );
        drop(token2);
        assert_eq!(
            root.count(),
            0,
            "Node count did not decrease by 1 when a cloned token was dropped"
        );
    }

    #[test]
    fn linear_tree() {
        let root = TreeCount::new();
        let parent = root.new_child();
        let child = parent.new_child();
        assert_eq!(root.count(), 0, "Initial count of root node is not 0");
        assert_eq!(parent.count(), 0, "Initial count of parent node is not 0");
        assert_eq!(child.count(), 0, "Initial count of child node is not 0");
        let root_token = root.get_token();
        assert_eq!(
            root.count(),
            1,
            "Root node count did not increment for a token"
        );
        assert_eq!(
            parent.count(),
            0,
            "Parent node count incremented for root token"
        );
        assert_eq!(
            child.count(),
            0,
            "Child node count incremented for root token"
        );
        let _child_token = child.get_token();
        assert_eq!(
            root.count(),
            2,
            "Root node count did not increment for child token"
        );
        assert_eq!(
            parent.count(),
            1,
            "Parent node count did not increment for child token"
        );
        assert_eq!(
            child.count(),
            1,
            "Child node count did not increment for child token"
        );
        drop(root_token);
        assert_eq!(
            root.count(),
            1,
            "Root node count did not decrement for root token drop"
        );
        assert_eq!(
            parent.count(),
            1,
            "Parent node count decremented for root token drop"
        );
        assert_eq!(
            child.count(),
            1,
            "CHild node count decremented for root token drop"
        );
    }

    #[test]
    fn branched_tree() {
        let root = TreeCount::new();
        let left = root.new_child();
        let right = root.new_child();
        assert_eq!(root.count(), 0, "Initial count of root node is not 0");
        assert_eq!(left.count(), 0, "Initial count of left node is not 0");
        assert_eq!(right.count(), 0, "Initial count of right node is not 0");
        let _left_token = left.get_token();
        assert_eq!(root.count(), 1, "Root count not incremented for left token");
        assert_eq!(left.count(), 1, "Left count not incremented for own token");
        assert_eq!(right.count(), 0, "Right count incremented for left token");
        let _right_token = right.get_token();
        assert_eq!(
            root.count(),
            2,
            "Root count not incremented for right token"
        );
        assert_eq!(left.count(), 1, "Left count incremented for right token");
        assert_eq!(
            right.count(),
            1,
            "Right count not incremented for own token"
        );
    }

    #[tokio::test]
    async fn ready_no_wait() {
        let root = TreeCount::new();
        let _token = root.get_token();
        let count = timeout(Duration::from_secs(0), root).await;
        assert!(
            count.is_ok(),
            "Waiting on a tree with an outstanding token waited"
        );
        assert_eq!(
            count.unwrap(),
            1,
            "Incorrect count for tree with outstanding token"
        );
    }

    #[tokio::test]
    async fn timeout_wait() {
        let root = TreeCount::new();
        let root_token_source = root.clone();
        let (count, _) = tokio::join!(
            tokio::spawn(async move { timeout(Duration::from_millis(100), root).await }),
            tokio::spawn(async move {
                sleep(Duration::from_millis(300)).await;
                root_token_source.get_token()
            })
        );
        let count = count.expect("inner tokio task failed");
        assert!(
            count.is_err(),
            "Waiting for a tree to become ready was successful when it shouldn't be"
        );
    }

    //#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[tokio::test]
    async fn simple_ready_wait() {
        let root = TreeCount::new();
        let root_token_source = root.clone();
        let start_time = Instant::now();
        let (count, _) = tokio::join!(
            tokio::spawn(async move { timeout(Duration::from_millis(1000), root).await }),
            tokio::spawn(async move {
                sleep(Duration::from_millis(100)).await;
                root_token_source.get_token()
            })
        );
        let elapsed_millis = start_time.elapsed().as_millis();
        // The actual timeout is 1s, but the task should be woken after 100ms. Give some leeway and
        // consider anything under 500ms as good.
        assert!(
            elapsed_millis < 500,
            "Waiting task wasn't woken (after {}ms)",
            elapsed_millis
        );
        let count = count.expect("inner tokio task failed");
        assert!(count.is_ok(), "Waiting for a tree to become ready failed");
        assert_eq!(count.unwrap(), 1, "Incorrect count for tree");
    }

    //#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[tokio::test]
    async fn linear_ready_wait() {
        let root = TreeCount::new();
        let child = root.new_child();
        let child_token_source = child.clone();
        let start_time = Instant::now();
        let tasks = tokio::join!(
            tokio::spawn(async move {
                timeout(Duration::from_millis(1000), root)
                    .await
                    .expect("root wait timed out");
                Instant::now()
            }),
            tokio::spawn(async move {
                timeout(Duration::from_millis(1000), child)
                    .await
                    .expect("child wait timed out");
                Instant::now()
            }),
            tokio::spawn(async move {
                sleep(Duration::from_millis(100)).await;
                let token = child_token_source.get_token();
                // Dance around and give the other tasks a chance to run.
                let token_time = Instant::now();
                sleep(Duration::from_millis(400)).await;
                (token_time, token)
            }),
        );
        let total_elapsed = start_time.elapsed();
        // The actual timeout is 1s, but everything should be done after 500ms.
        assert!(
            total_elapsed.as_millis() < 700,
            "Waiting tasks weren't woken (after {}ms)",
            total_elapsed.as_millis()
        );
        let root_wake = tasks.0.expect("Root wait task panicked");
        let child_wake = tasks.1.expect("Child wait task panicked");
        let (token_time, _token) = tasks.2.expect("Token acquisition task panicked");
        assert!(
            root_wake < child_wake,
            "Child node was awoken before root node"
        );
        assert!(
            token_time < root_wake,
            "Token time was somehow after root wake time."
        );
        assert!(
            token_time < child_wake,
            "Token time was somehow after child wake time."
        );
    }
}
