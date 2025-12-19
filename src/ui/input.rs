//! Input handling for the TUI.
//!
//! This module handles keyboard input and translates it into application actions.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

/// Actions that can be performed based on user input
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    /// Quit the application
    Quit,
    /// Toggle measurement (start/stop)
    ToggleMeasurement,
    /// Reload configuration
    Reload,
    /// Select next sensor
    NextSensor,
    /// Select previous sensor
    PrevSensor,
    /// Toggle selected sensor enabled/disabled
    ToggleSensor,
    /// Clear chart data
    ClearCharts,
    /// Toggle help panel
    ToggleHelp,
    /// No action
    None,
}

/// Handle keyboard input and return the corresponding action
pub fn handle_input(timeout: Duration) -> std::io::Result<InputAction> {
    if event::poll(timeout)? {
        if let Event::Key(key_event) = event::read()? {
            return Ok(map_key_to_action(key_event));
        }
    }
    Ok(InputAction::None)
}

/// Map a key event to an application action
fn map_key_to_action(key_event: KeyEvent) -> InputAction {
    match key_event.code {
        // Quit
        KeyCode::Char('q') | KeyCode::Char('Q') => InputAction::Quit,
        KeyCode::Esc => InputAction::Quit,
        KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            InputAction::Quit
        }

        // Toggle measurement
        KeyCode::Char(' ') => InputAction::ToggleMeasurement,
        KeyCode::Char('p') | KeyCode::Char('P') => InputAction::ToggleMeasurement,

        // Reload
        KeyCode::Char('r') | KeyCode::Char('R') => InputAction::Reload,

        // Navigate sensors
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => InputAction::PrevSensor,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => InputAction::NextSensor,

        // Toggle sensor
        KeyCode::Char('d') | KeyCode::Char('D') => InputAction::ToggleSensor,
        KeyCode::Enter => InputAction::ToggleSensor,

        // Clear charts
        KeyCode::Char('c') | KeyCode::Char('C') => InputAction::ClearCharts,

        // Help
        KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::F(1) => {
            InputAction::ToggleHelp
        }

        _ => InputAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quit_actions() {
        assert_eq!(
            map_key_to_action(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::empty())),
            InputAction::Quit
        );
        assert_eq!(
            map_key_to_action(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())),
            InputAction::Quit
        );
    }

    #[test]
    fn test_navigation() {
        assert_eq!(
            map_key_to_action(KeyEvent::new(KeyCode::Up, KeyModifiers::empty())),
            InputAction::PrevSensor
        );
        assert_eq!(
            map_key_to_action(KeyEvent::new(KeyCode::Down, KeyModifiers::empty())),
            InputAction::NextSensor
        );
    }
}
