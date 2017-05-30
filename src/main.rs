#[macro_use]
extern crate futures;
extern crate tokio_core;

use std::thread;

use tokio_core::reactor::Core;
use futures::future::*;
use futures::sync::mpsc;
use futures::stream::Fuse;
use futures::{Stream, Sink, StartSend, Poll, Async, AsyncSink, Future};

fn main() {
    println!("Hello, world!");

    let mut core = Core::new().unwrap();

    let (mut tx, rx) = mpsc::channel(1);

    thread::spawn(|| {
        for i in 0..5 {
            tx = tx.send(i + 1).wait().unwrap();
        }
    });

    //let result = rx.and_then(|i| {
    //    println!("{}", i); Ok(())
    //}).collect();
    let sinksstream = MySinksStream;
    let result = sinksstream.fold(rx, |rx, sink| {
        //println!("new sink");
        new(sink, rx).then(|result| {
            let (_, rx, reason) = result.unwrap();
            match reason {
                Reason::StreamEnded => Err(()),
                Reason::SinkEnded => Ok(rx),
            }
        })
        //sink.send_all(rx).map(|(_, rx)| rx).or_else(|_| Ok(()))
        //Ok(rx)
    });

    core.run(result).unwrap();
}

struct MySink {
    sent: u32,
}

impl Sink for MySink {
    type SinkItem = u32;
    type SinkError = ();

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        self.sent += 1;
        println!("item: {} (total to this sink: {}", item, self.sent);
        if self.sent > 2 {
            Err(())
        } else {
            Ok(AsyncSink::Ready)
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        //println!("sent: {}", self.sent);
        Ok(Async::Ready(()))
    }
}

struct MySinksStream;

impl Stream for MySinksStream {
    type Item = MySink;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        Ok(Async::Ready(Some(MySink {sent: 0})))
    }
}


//use {Poll, Async, Future, AsyncSink};
//use stream::{Stream, Fuse};
//use sink::Sink;

/// Future for the `Sink::send_all` combinator, which sends a stream of values
/// to a sink and then waits until the sink has fully flushed those values.
#[derive(Debug)]
#[must_use = "futures do nothing unless polled"]
pub struct SendAll<T, U: Stream> {
    sink: Option<T>,
    stream: Option<Fuse<U>>,
    buffered: Option<U::Item>,
}

pub fn new<T, U>(sink: T, stream: U) -> SendAll<T, U>
    where T: Sink,
          U: Stream<Item = T::SinkItem>,
          T::SinkError: From<U::Error>,
{
    SendAll {
        sink: Some(sink),
        stream: Some(stream.fuse()),
        buffered: None,
    }
}

pub enum Reason {
    StreamEnded,
    SinkEnded,
}

impl<T, U> SendAll<T, U>
    where T: Sink,
          U: Stream<Item = T::SinkItem>,
          T::SinkError: From<U::Error>,
{
    fn sink_mut(&mut self) -> &mut T {
        self.sink.as_mut().take().expect("Attempted to poll SendAll after completion")
    }

    fn stream_mut(&mut self) -> &mut Fuse<U> {
        self.stream.as_mut().take()
            .expect("Attempted to poll SendAll after completion")
    }

    fn take_result(&mut self, reason: Reason) -> (T, U, Reason) {
        let sink = self.sink.take()
            .expect("Attempted to poll Forward after completion");
        let fuse = self.stream.take()
            .expect("Attempted to poll Forward after completion");
        return (sink, fuse.into_inner(), reason);
    }

    fn try_start_send(&mut self, item: U::Item) -> Poll<(), T::SinkError> {
        debug_assert!(self.buffered.is_none());
        if let AsyncSink::NotReady(item) = try!(self.sink_mut().start_send(item)) {
            self.buffered = Some(item);
            return Ok(Async::NotReady)
        }
        Ok(Async::Ready(()))
    }
}

impl<T, U> Future for SendAll<T, U>
    where T: Sink,
          U: Stream<Item = T::SinkItem>,
          T::SinkError: From<U::Error>,
{
    type Item = (T, U, Reason);
    type Error = T::SinkError;

    fn poll(&mut self) -> Poll<(T, U, Reason), T::SinkError> {
        // If we've got an item buffered already, we need to write it to the
        // sink before we can do anything else
        if let Some(item) = self.buffered.take() {
            try_ready!(self.try_start_send(item))
        }

        loop {
            match try!(self.stream_mut().poll()) {
                Async::Ready(Some(item)) => {
                    match self.try_start_send(item) {
                        Ok(Async::Ready(t)) => t,
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Err(e) => {
                            try_ready!(self.sink_mut().close());
                            return Ok(Async::Ready(self.take_result(Reason::SinkEnded)))
                        },
                    }
                },
                Async::Ready(None) => {
                    try_ready!(self.sink_mut().close());
                    return Ok(Async::Ready(self.take_result(Reason::StreamEnded)))
                }
                Async::NotReady => {
                    try_ready!(self.sink_mut().poll_complete());
                    return Ok(Async::NotReady)
                }
            }
        }
    }
}
