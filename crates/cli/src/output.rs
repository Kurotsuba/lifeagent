//! Small print helpers shared by the subcommands.

use anyhow::Result;
use lifeagent_core::app::ChatTurn;
use serde::Serialize;

/// A `send_message` turn's reply, then each activity on its own bullet line — what `chat`
/// and `say` print (§11).
pub fn print_turn(turn: &ChatTurn) {
    println!("{}", turn.reply);
    for activity in &turn.activities {
        println!("  · {activity}");
    }
}

/// Every other subcommand's output: pretty-printed JSON, so results stay both readable and
/// scriptable for a developer harness.
pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
