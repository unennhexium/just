use super::*;

use nix::unistd::Pid;

pub(crate) struct SignalHandler {
  caught: Option<Signal>,
  children: BTreeMap<Pid, Command>,
  verbosity: Verbosity,
}

impl SignalHandler {
  pub(crate) fn install(verbosity: Verbosity) -> RunResult<'static> {
    let mut instance = Self::instance();
    instance.verbosity = verbosity;
    Platform::install_signal_handler(|signal| Self::instance().interrupt(signal))
  }

  pub(crate) fn instance() -> MutexGuard<'static, Self> {
    static INSTANCE: Mutex<SignalHandler> = Mutex::new(SignalHandler::new());

    match INSTANCE.lock() {
      Ok(guard) => guard,
      Err(poison_error) => {
        eprintln!(
          "{}",
          Error::Internal {
            message: format!("signal handler mutex poisoned: {poison_error}"),
          }
          .color_display(Color::auto().stderr())
        );
        process::exit(EXIT_FAILURE);
      }
    }
  }

  const fn new() -> Self {
    Self {
      caught: None,
      children: BTreeMap::new(),
      verbosity: Verbosity::default(),
    }
  }

  fn interrupt(&mut self, signal: Signal) {
    if self.children.is_empty() {
      process::exit(128 + signal.number());
    }

    if signal != Signal::Info && self.caught.is_none() {
      self.caught = Some(signal);
    }

    match signal {
      // SIGTERM is the default signal sent by kill. forward it to child
      // processes and wait for them to exit
      Signal::Terminate => {
        for &child in self.children.keys() {
          eprintln!("sending sigterm to {child}");
          nix::sys::signal::kill(child, Some(Signal::Terminate.into()));
        }
      }
      Signal::Info => {
        // todo: print pid
        if self.children.is_empty() {
          eprintln!("just: no child processes");
        } else {
          // todo: handle plural correctly
          let mut message = format!("just: {} child processes:\n", self.children.len());

          for (&child, command) in &self.children {
            message.push_str(&format!("{child}: {command:?}\n"));
          }

          eprint!("{message}");
        }
      }
      // SIGHUP, SIGINT, and SIGQUIT are normally sent on terminal close,
      // ctrl-c, and ctrl-\, respectively, and are sent to all processes in the
      // foreground process group. this includes child processes, so we ignore
      // the signal and rely on them to exit
      Signal::Hangup | Signal::Interrupt | Signal::Quit => {}
    }
  }

  pub(crate) fn spawn<T>(
    mut command: Command,
    f: impl Fn(process::Child) -> io::Result<T>,
  ) -> (io::Result<T>, Option<Signal>) {
    let mut instance = Self::instance();

    let child = match command.spawn() {
      Err(err) => return (Err(err), None),
      Ok(child) => child,
    };

    let pid = Pid::from_raw(child.id() as i32);

    instance.children.insert(pid, command);

    drop(instance);

    let result = f(child);

    let mut instance = Self::instance();

    instance.children.remove(&pid);

    (result, instance.caught)
  }
}
