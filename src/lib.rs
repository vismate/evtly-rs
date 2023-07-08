use std::{any::Any, ops::Deref, sync::Arc, time::Duration};

#[cfg(feature = "global-instance")]
lazy_static::lazy_static! {
    static ref GLOBAL_EVENT_BUS: EventBus = {
        EventBus::default()
    };
}

/// Describes whether the event should be propagated to other handlers further down the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPropagation {
    /// Propagate the event to future handlers down the line.
    Propagate,
    /// Stop propagation of this event.
    Consume,
    /// Let other handlers on this layer handle this event, but don't let it propagate to further layers.
    DeferConsume,
}

impl Default for EventPropagation {
    fn default() -> Self {
        Self::Propagate
    }
}

type LayerIndex = u32;

/// Indicates what happened to the posted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPostResult {
    /// There were no event handlers registered for this event, so it was not handled.
    Unhandled,
    /// The event was handled by all registered event handlers for this type of event and was never consumed in any way.
    FellThrough,
    /// Some event handler consumed the event, stopping its propagation immidiately.
    ConsumedImmidiately(LayerIndex),
    /// Some event handler consumed the event, but let it propagate to other handlers on the same layer.
    ConsumedOnLayer(LayerIndex),
}

/// Handler for events of type T.
pub trait EventHandler<T: ?Sized>: Send + Sync {
    /// Handles the event of type T.
    ///
    /// Returns the propagation result indicating whether the event should be propagated further or consumed.
    fn handle_event(&self, event: &T) -> EventPropagation;
}

// This implementation makes using smart pointers (eg.: Arc) as handlers much easier
impl<T, H, DH> EventHandler<T> for DH
where
    T: ?Sized,
    H: EventHandler<T>,
    DH: Deref<Target = H> + Send + Sync,
{
    fn handle_event(&self, event: &T) -> EventPropagation {
        self.deref().handle_event(event)
    }
}

/// Wrapper for an event handling closure or fn
pub struct HandlerFn<F>(pub F);

impl<F> HandlerFn<F> {
    pub fn new(f: F) -> Arc<Self> {
        Arc::new(HandlerFn(f))
    }
}

impl<T: ?Sized, F> EventHandler<T> for HandlerFn<F>
where
    for<'a> F: Fn(&'a T) -> EventPropagation + Send + Sync,
{
    fn handle_event(&self, event: &T) -> EventPropagation {
        (self.0)(event)
    }
}

struct HandlerWithMeta<T: ?Sized> {
    layer: LayerIndex,
    handler: Arc<dyn EventHandler<T>>,
}

type Handlers<T> = Vec<HandlerWithMeta<T>>;

#[derive(Debug)]
pub struct TimeoutError {
    timeout: Duration,
}

impl std::fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "EventBus error: timeout = {}ms",
            self.timeout.as_millis()
        )
    }
}

impl std::error::Error for TimeoutError {}

/// Event bus used to register handlers and post events.
#[derive(Debug, Default)]
pub struct EventBus {
    handlers: parking_lot::RwLock<anymap::Map<dyn Any + Send + Sync>>,
}

impl EventBus {
    const DEFAULT_TIMEOUT: Duration = Duration::from_millis(10);

    /// Posts an event to event handlers registered for the specified event type.
    ///
    /// Returns the result indicating what happened to the posted event.
    pub fn post<T: ?Sized + 'static>(&self, event: &T) -> EventPostResult {
        match self.handlers.read().get::<Handlers<T>>() {
            Some(handlers) if !handlers.is_empty() => {
                let mut consume_on_layer = None;

                for h in handlers {
                    if consume_on_layer.is_some_and(|consume_on| consume_on != h.layer) {
                        return EventPostResult::ConsumedOnLayer(consume_on_layer.unwrap());
                    }

                    match h.handler.handle_event(event) {
                        EventPropagation::Consume => {
                            return EventPostResult::ConsumedImmidiately(h.layer);
                        }
                        EventPropagation::DeferConsume => {
                            consume_on_layer = Some(h.layer);
                        }
                        EventPropagation::Propagate => continue,
                    }
                }

                // Technically the event was seen by all handlers, but we do this to be consistent with the ConsumeImmidiately case.
                consume_on_layer.map_or(EventPostResult::FellThrough, |consume_on| {
                    EventPostResult::ConsumedOnLayer(consume_on)
                })
            }

            _ => EventPostResult::Unhandled,
        }
    }

    /// Registers an event handler for a specific type of event on the specified layer.
    pub fn register_with_timeout<T: ?Sized + 'static>(
        &self,
        handler: Arc<impl EventHandler<T> + 'static>,
        layer: LayerIndex,
        timeout: Duration,
    ) -> Result<(), TimeoutError> {
        let mut map = self
            .handlers
            .try_write_for(timeout)
            .ok_or(TimeoutError { timeout })?;

        let handlers = map
            .entry::<Handlers<T>>()
            .or_insert(Handlers::<T>::default());

        let position = handlers
            .binary_search_by_key(&layer, |h| h.layer)
            .unwrap_or_else(|e| e);

        handlers.insert(position, HandlerWithMeta { layer, handler });

        Ok(())
    }

    #[inline]
    pub fn register<T: ?Sized + 'static>(
        &self,
        handler: Arc<impl EventHandler<T> + 'static>,
        layer: LayerIndex,
    ) -> Result<(), TimeoutError> {
        self.register_with_timeout(handler, layer, Self::DEFAULT_TIMEOUT)
    }

    /// Removes all registered event handlers.
    /// Returns an error if could not lock the handlers at the moment
    pub fn remove_all_handlers_with_timeout(&self, timeout: Duration) -> Result<(), TimeoutError> {
        self.handlers
            .try_write_for(timeout)
            .ok_or(TimeoutError { timeout })?
            .clear();
        Ok(())
    }

    #[inline]
    pub fn remove_all_handlers(&self) -> Result<(), TimeoutError> {
        self.remove_all_handlers_with_timeout(Self::DEFAULT_TIMEOUT)
    }

    /// Removes event handlers for the specified event type.
    /// Returns an error if could not lock the handlers at the moment
    pub fn remove_handlers_of_with_timeout<T: ?Sized + 'static>(
        &self,
        timeout: Duration,
    ) -> Result<(), TimeoutError> {
        self.handlers
            .try_write_for(timeout)
            .ok_or(TimeoutError { timeout })?
            .remove::<Handlers<T>>();
        Ok(())
    }

    #[inline]
    pub fn remove_handlers_of<T: ?Sized + 'static>(&self) -> Result<(), TimeoutError> {
        self.remove_handlers_of_with_timeout::<T>(Self::DEFAULT_TIMEOUT)
    }

    /// Remove a specific handler identified by the uuid return by the register method
    pub fn remove_handler_with_timeout<T: ?Sized + 'static>(
        &self,
        handler: &Arc<impl EventHandler<T> + 'static>,
        timeout: Duration,
    ) -> Result<(), TimeoutError> {
        // TODO: Mayber there is a nicer way? Arc::ptr_eq requires same Ts for both handlers.
        // One is impl EvenHandler<T> the others is dyn EventHandler<T>
        let handler_ptr = Arc::as_ptr(handler) as *const ();
        if let Some(handlers) = self
            .handlers
            .try_write_for(timeout)
            .ok_or(TimeoutError { timeout })?
            .get_mut::<Handlers<T>>()
        {
            handlers.retain(|other| {
                !std::ptr::eq(handler_ptr, Arc::as_ptr(&other.handler) as *const ())
            });
        }

        Ok(())
    }

    #[inline]
    pub fn remove_handler<T: ?Sized + 'static>(
        &self,
        handler: &Arc<impl EventHandler<T> + 'static>,
    ) -> Result<(), TimeoutError> {
        self.remove_handler_with_timeout(handler, Self::DEFAULT_TIMEOUT)
    }

    /// Index of the very first layer possible
    pub const fn foreground_layer() -> LayerIndex {
        LayerIndex::MIN
    }

    /// Index of the very last layer possible
    pub const fn background_layer() -> LayerIndex {
        LayerIndex::MAX
    }

    #[cfg(feature = "global-instance")]
    pub fn global() -> &'static Self {
        &GLOBAL_EVENT_BUS
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

    fn basic_test(eb: &EventBus) {
        let handler = Arc::new(FooBarHandler);

        let _ = eb.register::<FooEvent>(handler.clone(), 1);
        let _ = eb.register::<BarEvent>(handler, 1);
        let to_remove = HandlerFn::new(|_evt: &i32| EventPropagation::Consume);
        let _ = eb.register(to_remove.clone(), 1);

        assert!(eb.remove_handler(&to_remove).is_ok());

        let test = || {
            assert_eq!(eb.post("Hello"), EventPostResult::Unhandled);
            assert_eq!(eb.post(&42i32), EventPostResult::Unhandled);
            assert_eq!(eb.post(&FooEvent), EventPostResult::FellThrough);

            assert_eq!(eb.post(&BarEvent), EventPostResult::ConsumedImmidiately(1));
        };

        std::thread::scope(|s| {
            s.spawn(test);
            s.spawn(test);
        });
    }

    #[test]
    fn it_works() {
        let eb = EventBus::default();
        basic_test(&eb);
        #[cfg(feature = "global-instance")]
        {
            let eb = EventBus::global();
            basic_test(&eb);
        }
    }

    #[test]
    fn try_mut_eb_in_handler() {
        let eb = EventBus::default();

        // We post the eventbus itself to avoid using the global feature
        let _ = eb.register(
            HandlerFn::new(|eb: &EventBus| {
                let res = eb.remove_all_handlers();
                assert!(res.is_err());
                EventPropagation::default()
            }),
            1,
        );

        eb.post(&eb);
    }

    #[test]
    fn dyn_traits() {
        let eb = EventBus::default();
        let _ = eb.register::<dyn std::fmt::Debug>(
            HandlerFn::new(|evt: &(dyn std::fmt::Debug + '_)| {
                println!("{evt:?}");
                EventPropagation::Propagate
            }),
            EventBus::foreground_layer(),
        );

        let eb_ref = &eb;

        eb.post(eb_ref as &dyn std::fmt::Debug);
    }
}
