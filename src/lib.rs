use std::{
    any::{Any, TypeId},
    cmp::Reverse,
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// Describes whether the event should be propagated to other handlers further down the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPropagation {
    /// Propagate the event to future handlers down the line.
    Propagate,
    /// Stop propagation of this event.
    Consume,
}

/// Indicates what happened to the posted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPostResult {
    /// There were no event handlers registered for this event, so it was not handled.
    Unhandled,
    /// The event was handled by all registered event handlers for this type of event.
    Fallthrough,
    /// Some event handler consumed the event, stopping its propagation.
    Consumed,
}

/// Handler for events of type T.
pub trait EventHandler<T: ?Sized>: Send + Sync {
    /// Handles the event of type T.
    ///
    /// Returns the propagation result indicating whether the event should be propagated further or consumed.
    fn handle_event(&self, event: &T) -> EventPropagation;
}

impl<F, T> EventHandler<T> for F
where
    T: ?Sized,
    F: Send + Sync + Fn(&T) -> EventPropagation,
{
    fn handle_event(&self, event: &T) -> EventPropagation {
        (self)(event)
    }
}

/// Event bus used to register handlers and post events.
#[derive(Debug, Default)]
pub struct EventBus {
    handlers: RwLock<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

type Handlers<T> = Vec<(Reverse<u32>, Box<dyn EventHandler<T>>)>;

impl EventBus {
    /// Posts an event to event handlers registered for the specified event type.
    ///
    /// Returns the result indicating what happened to the posted event.
    pub fn post<T: ?Sized + 'static>(&self, event: &T) -> EventPostResult {
        if let Some(handlers) = self
            .handlers
            .read()
            .expect("could lock map for reading")
            .get(&TypeId::of::<T>())
        {
            let handlers = handlers
                .downcast_ref::<Handlers<T>>()
                .expect("handlers for event type");

            for (_, handler) in handlers {
                match handler.handle_event(event) {
                    EventPropagation::Consume => {
                        return EventPostResult::Consumed;
                    }
                    EventPropagation::Propagate => continue,
                }
            }

            EventPostResult::Fallthrough
        } else {
            EventPostResult::Unhandled
        }
    }

    /// Registers an event handler for a specific type of event on the specified layer.
    pub fn register<T: ?Sized + 'static>(&self, handler: Box<dyn EventHandler<T>>, layer: u32) {
        let mut map = self.handlers.write().expect("could lock map for writing");

        let handlers = map
            .entry(TypeId::of::<T>())
            .or_insert(Box::new(Handlers::<T>::default()))
            .downcast_mut::<Handlers<T>>()
            .expect("correctly typed handler stored");

        let layer = Reverse(layer);

        let position = handlers
            .binary_search_by_key(&layer, |(p, _)| *p)
            .unwrap_or_else(|e| e);

        handlers.insert(position, (layer, handler));
    }

    /// Removes all registered event handlers.
    pub fn remove_all_handlers(&self) {
        self.handlers
            .write()
            .expect("could lock map for writing")
            .clear();
    }

    /// Removes event handlers for the specified event type.
    pub fn remove_handlers_of<T: ?Sized + 'static>(&self) {
        self.handlers
            .write()
            .expect("could lock map for writing")
            .remove(&TypeId::of::<T>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FooEvent;
    struct BarEvent;

    struct FooBarHandler;

    impl EventHandler<FooEvent> for FooBarHandler {
        fn handle_event(&self, _event: &FooEvent) -> EventPropagation {
            println!("FooEvent handeled");
            EventPropagation::Propagate
        }
    }

    impl EventHandler<BarEvent> for FooBarHandler {
        fn handle_event(&self, _event: &BarEvent) -> EventPropagation {
            println!("BarEvent handeled");
            EventPropagation::Consume
        }
    }

    #[test]
    fn it_works() {
        let eb = Arc::new(EventBus::default());

        let eb1 = eb.clone();
        let eb2 = eb.clone();

        std::thread::spawn(move || {
            eb1.register::<FooEvent>(Box::new(FooBarHandler), 1);
            eb1.register::<BarEvent>(Box::new(FooBarHandler), 1);

            assert_eq!(eb1.post("Hello"), EventPostResult::Unhandled);
            assert_eq!(eb1.post(&FooEvent), EventPostResult::Fallthrough);

            assert_eq!(eb1.post(&BarEvent), EventPostResult::Consumed);
        });

        std::thread::spawn(move || {
            eb2.register::<FooEvent>(Box::new(FooBarHandler), 1);
            eb2.register::<BarEvent>(Box::new(FooBarHandler), 1);

            assert_eq!(eb2.post("Hello"), EventPostResult::Unhandled);
            assert_eq!(eb2.post(&FooEvent), EventPostResult::Fallthrough);

            assert_eq!(eb2.post(&BarEvent), EventPostResult::Consumed);
        });
    }
}
