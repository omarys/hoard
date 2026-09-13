use crossbeam_channel::unbounded;
use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use std::thread;
use std::time::Duration;

pub enum Event {
    Input(KeyEvent),
    Tick,
}

/// A small event handler that wraps crossterm input and tick events. Each
/// event type is handled in its own thread and returned to a common `Receiver`.
pub struct Events {
    rx: crossbeam_channel::Receiver<Event>,
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub tick_rate: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tick_rate: Duration::from_millis(250),
        }
    }
}

impl Events {
    pub fn with_config(config: Config) -> Self {
        let (tx, rx) = unbounded();

        {
            let tx = tx.clone();
            thread::spawn(move || {
                loop {
                    match event::read() {
                        Ok(CrosstermEvent::Key(key)) => {
                            if tx.send(Event::Input(key)).is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(err) => {
                            eprintln!("{err}");
                            break;
                        }
                    }
                }
            });
        }

        thread::spawn(move || {
            loop {
                if tx.send(Event::Tick).is_err() {
                    break;
                }
                thread::sleep(config.tick_rate);
            }
        });
        Self { rx }
    }

    pub fn next(&self) -> Result<Event, crossbeam_channel::RecvError> {
        self.rx.recv()
    }
}
