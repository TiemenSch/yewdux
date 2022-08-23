#[cfg(feature = "future")]
use std::future::Future;
use std::rc::Rc;

use anymap::AnyMap;

use crate::{mrc::Mrc, store::Store};

pub(crate) struct Context<S> {
    pub(crate) store: Mrc<Rc<S>>,
}

impl<S> Clone for Context<S> {
    fn clone(&self) -> Self {
        Self {
            store: Mrc::clone(&self.store),
        }
    }
}

impl<S: Store> Context<S> {
    /// Apply a function to state, returning if it should notify subscribers or not.
    pub(crate) fn reduce(&self, f: impl FnOnce(Rc<S>) -> Rc<S>) -> bool {
        let old = Rc::clone(&self.store.borrow());
        *self.store.borrow_mut() = f(Rc::clone(&old));

        self.store.borrow().should_notify(&old)
    }

    /// Apply a future reduction to state, returning if it should notify subscribers or not.
    #[cfg(feature = "future")]
    pub(crate) async fn reduce_future<FUN, FUT>(&self, f: FUN) -> bool
    where
        FUN: FnOnce(Rc<S>) -> FUT,
        FUT: Future<Output = Rc<S>>,
    {
        let old = Rc::clone(&self.store.borrow());

        *self.store.borrow_mut() = f(Rc::clone(&old)).await;

        self.store.borrow().should_notify(&old)
    }
}

pub(crate) fn get_or_init<S: Store>() -> Context<S> {
    thread_local! {
        /// Holds all shared state.
        static CONTEXTS: Mrc<AnyMap> = Mrc::new(AnyMap::new());
    }

    let contexts = CONTEXTS
        .try_with(|contexts| contexts.clone())
        .expect("CONTEXTS thread local key init failed");

    // Get context, or None if it doesn't exist.
    //
    // We use an option here because a new Store should not be created during this borrow. We want
    // to allow this store access to other stores during creation, so cannot be borrowing the
    // global resource while initializing. Instead we create a temporary placeholder, which
    // indicates the store needs to be created. Without this indicator we would have needed to
    // check if the map contains the entry beforehand, which would have meant two map lookups per
    // call instead of just one.
    let maybe_context = contexts.with_mut(|x| {
        x.entry::<Mrc<Option<Context<S>>>>()
            .or_insert_with(|| None.into())
            .clone()
    });

    // If it doesn't exist, create and store the context (no pun intended).
    let exists = maybe_context.borrow().is_some();
    if !exists {
        // Init context outside of borrow. This allows the store to access other stores when it is
        // being created.
        let context = Context {
            store: Mrc::new(Rc::new(S::new())),
        };

        *maybe_context.borrow_mut() = Some(context);
    }

    // Now we get the context, which must be initialized because we already checked above.
    let context = maybe_context
        .borrow()
        .clone()
        .expect("Context not initialized");

    context
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[derive(Clone, PartialEq, Eq)]
    struct TestState(u32);
    impl Store for TestState {
        fn new() -> Self {
            Self(0)
        }

        fn should_notify(&self, other: &Self) -> bool {
            self != other
        }
    }

    #[derive(Clone, PartialEq, Eq)]
    struct TestState2(u32);
    impl Store for TestState2 {
        fn new() -> Self {
            get_or_init::<TestState>();
            Self(0)
        }

        fn should_notify(&self, other: &Self) -> bool {
            self != other
        }
    }

    #[test]
    fn can_access_other_store_for_new_of_current_store() {
        let _context = get_or_init::<TestState2>();
    }

    #[derive(Clone, PartialEq, Eq)]
    struct StoreNewIsOnlyCalledOnce(Rc<Cell<u32>>);
    impl Store for StoreNewIsOnlyCalledOnce {
        fn new() -> Self {
            thread_local! {
                /// Stores all shared state.
                static COUNT: Rc<Cell<u32>> = Default::default();
            }

            let count = COUNT.try_with(|x| x.clone()).unwrap();

            count.set(count.get() + 1);

            Self(count)
        }

        fn should_notify(&self, other: &Self) -> bool {
            self != other
        }
    }

    #[test]
    fn store_new_is_only_called_once() {
        get_or_init::<StoreNewIsOnlyCalledOnce>();
        let context = get_or_init::<StoreNewIsOnlyCalledOnce>();

        assert!(context.store.borrow().0.get() == 1)
    }
}
