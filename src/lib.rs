use std::{
    any::{Any, TypeId},
    cell::RefCell,
    cmp::Reverse,
    collections::HashMap,
    rc::Rc,
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
pub trait EventHandler<T: ?Sized> {
    /// Handles the event of type T.
    ///
    /// Returns the propagation result indicating whether the event should be propagated further or consumed.
    fn handle_event(&mut self, event: &T) -> EventPropagation;
}

impl<F, T> EventHandler<T> for F
where
    T: ?Sized,
    F: FnMut(&T) -> EventPropagation,
{
    fn handle_event(&mut self, event: &T) -> EventPropagation {
        (self)(event)
    }
}

/// Event bus used to register handlers and post events.
#[derive(Debug, Default)]
pub struct EventBus {
    handlers: HashMap<TypeId, Box<dyn Any>>,
}

type EventHandlers<T> = Vec<(Reverse<u32>, Box<dyn EventHandler<T>>)>;

impl EventBus {
    /// Posts an event to event handlers registered for the specified event type.
    ///
    /// Returns the result indicating what happened to the posted event.
    pub fn post<T: ?Sized + 'static>(&mut self, event: &T) -> EventPostResult {
        if let Some(handlers) = self.handlers.get_mut(&TypeId::of::<T>()) {
            let handlers = handlers
                .downcast_mut::<EventHandlers<T>>()
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
    pub fn register<T: ?Sized + 'static>(&mut self, handler: Box<dyn EventHandler<T>>, layer: u32) {
        let handlers = self
            .handlers
            .entry(TypeId::of::<T>())
            .or_insert(Box::new(EventHandlers::<T>::default()))
            .downcast_mut::<EventHandlers<T>>()
            .expect("correctly typed handler stored");

        let layer = Reverse(layer);

        let position = handlers
            .binary_search_by_key(&layer, |(p, _)| *p)
            .unwrap_or_else(|e| e);

        handlers.insert(position, (layer, handler));
    }

    /// Removes all registered event handlers.
    pub fn remove_all_handlers(&mut self) {
        self.handlers.clear();
    }

    /// Removes event handlers for the specified event type.
    pub fn remove_handlers_of<T: ?Sized + 'static>(&mut self) {
        self.handlers.remove(&TypeId::of::<T>());
    }
}

impl<T: ?Sized + 'static, H: EventHandler<T>> EventHandler<T> for Rc<RefCell<H>> {
    fn handle_event(&mut self, event: &T) -> EventPropagation {
        self.borrow_mut().handle_event(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BuildEvent((usize, usize), u32);
    struct DestroyEvent(usize, usize);

    #[derive(Debug, Default)]
    struct World {
        tiles: [[Option<u32>; 3]; 3],
    }

    impl EventHandler<BuildEvent> for World {
        fn handle_event(&mut self, event: &BuildEvent) -> EventPropagation {
            let BuildEvent((x, y), tile) = event;
            self.tiles[*x][*y] = Some(*tile);
            EventPropagation::Propagate
        }
    }

    impl EventHandler<DestroyEvent> for World {
        fn handle_event(&mut self, event: &DestroyEvent) -> EventPropagation {
            let DestroyEvent(x, y) = event;
            self.tiles[*x][*y] = None;
            EventPropagation::Consume
        }
    }

    #[test]
    fn it_works() {
        let mut eb = EventBus::default();

        let world = Rc::new(RefCell::new(World::default()));

        eb.register::<BuildEvent>(Box::new(world.clone()), 1);
        eb.register::<DestroyEvent>(Box::new(world.clone()), 1);

        println!("{world:?}");

        assert_eq!(eb.post("Hello"), EventPostResult::Unhandled);
        assert_eq!(
            eb.post(&BuildEvent((2, 2), 3)),
            EventPostResult::Fallthrough
        );
        println!("{world:?}");
        assert_eq!(eb.post(&DestroyEvent(2, 2)), EventPostResult::Consumed);
        println!("{world:?}");
    }
}
