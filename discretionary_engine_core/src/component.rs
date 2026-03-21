// Stolen from nautilus_trader::common::component
// Lifecycle state machine for system components.

use std::fmt;

/// Components have state and lifecycle management capabilities.
pub trait Component: fmt::Debug {
	fn state(&self) -> ComponentState;
	fn transition_state(&mut self, trigger: ComponentTrigger);

	fn start(&mut self) {
		self.transition_state(ComponentTrigger::Start);
		self.on_start();
		self.transition_state(ComponentTrigger::StartCompleted);
	}

	fn stop(&mut self) {
		self.transition_state(ComponentTrigger::Stop);
		self.on_stop();
		self.transition_state(ComponentTrigger::StopCompleted);
	}

	fn resume(&mut self) {
		self.transition_state(ComponentTrigger::Resume);
		self.on_resume();
		self.transition_state(ComponentTrigger::ResumeCompleted);
	}

	fn reset(&mut self) {
		self.transition_state(ComponentTrigger::Reset);
		self.on_reset();
		self.transition_state(ComponentTrigger::ResetCompleted);
	}

	fn degrade(&mut self) {
		self.transition_state(ComponentTrigger::Degrade);
		self.on_degrade();
		self.transition_state(ComponentTrigger::DegradeCompleted);
	}

	fn fault(&mut self) {
		self.transition_state(ComponentTrigger::Fault);
		self.on_fault();
		self.transition_state(ComponentTrigger::FaultCompleted);
	}

	fn dispose(&mut self) {
		self.transition_state(ComponentTrigger::Dispose);
		self.on_dispose();
		self.transition_state(ComponentTrigger::DisposeCompleted);
	}

	fn on_start(&mut self) {}
	fn on_stop(&mut self) {}
	fn on_resume(&mut self) {}
	fn on_reset(&mut self) {}
	fn on_degrade(&mut self) {}
	fn on_fault(&mut self) {}
	fn on_dispose(&mut self) {}
}
/// The state of a component within the system.
#[derive(Clone, Copy, Debug, Default, strum::Display, Eq, Hash, PartialEq)]
pub enum ComponentState {
	#[default]
	PreInitialized,
	Ready,
	Starting,
	Running,
	Stopping,
	Stopped,
	Resuming,
	Resetting,
	Disposing,
	Disposed,
	Degrading,
	Degraded,
	Faulting,
	Faulted,
}
#[rustfmt::skip]
impl ComponentState {
	/// Transition the state machine with the given `trigger`.
	///
	/// # Panics
	///
	/// Panics if `trigger` is invalid for the current state.
	pub fn transition(&mut self, trigger: ComponentTrigger) -> Self {
		let new_state = match (&self, trigger) {
			(Self::PreInitialized, ComponentTrigger::Initialize) => Self::Ready,
			(Self::Ready, ComponentTrigger::Reset) => Self::Resetting,
			(Self::Ready, ComponentTrigger::Start) => Self::Starting,
			(Self::Ready, ComponentTrigger::Dispose) => Self::Disposing,
			(Self::Resetting, ComponentTrigger::ResetCompleted) => Self::Ready,
			(Self::Starting, ComponentTrigger::StartCompleted) => Self::Running,
			(Self::Starting, ComponentTrigger::Stop) => Self::Stopping,
			(Self::Starting, ComponentTrigger::Fault) => Self::Faulting,
			(Self::Running, ComponentTrigger::Stop) => Self::Stopping,
			(Self::Running, ComponentTrigger::Degrade) => Self::Degrading,
			(Self::Running, ComponentTrigger::Fault) => Self::Faulting,
			(Self::Resuming, ComponentTrigger::Stop) => Self::Stopping,
			(Self::Resuming, ComponentTrigger::ResumeCompleted) => Self::Running,
			(Self::Resuming, ComponentTrigger::Fault) => Self::Faulting,
			(Self::Stopping, ComponentTrigger::StopCompleted) => Self::Stopped,
			(Self::Stopping, ComponentTrigger::Fault) => Self::Faulting,
			(Self::Stopped, ComponentTrigger::Reset) => Self::Resetting,
			(Self::Stopped, ComponentTrigger::Resume) => Self::Resuming,
			(Self::Stopped, ComponentTrigger::Dispose) => Self::Disposing,
			(Self::Stopped, ComponentTrigger::Fault) => Self::Faulting,
			(Self::Degrading, ComponentTrigger::DegradeCompleted) => Self::Degraded,
			(Self::Degraded, ComponentTrigger::Resume) => Self::Resuming,
			(Self::Degraded, ComponentTrigger::Stop) => Self::Stopping,
			(Self::Degraded, ComponentTrigger::Fault) => Self::Faulting,
			(Self::Disposing, ComponentTrigger::DisposeCompleted) => Self::Disposed,
			(Self::Faulting, ComponentTrigger::FaultCompleted) => Self::Faulted,
			_ => panic!("Invalid state transition: {self} -> {trigger}"),
		};
		*self = new_state;
		new_state
	}
}

/// A trigger condition for a component state transition.
#[derive(Clone, Copy, Debug, strum::Display, Eq, Hash, PartialEq)]
pub enum ComponentTrigger {
	Initialize,
	Start,
	StartCompleted,
	Stop,
	StopCompleted,
	Resume,
	ResumeCompleted,
	Reset,
	ResetCompleted,
	Dispose,
	DisposeCompleted,
	Degrade,
	DegradeCompleted,
	Fault,
	FaultCompleted,
}
