//! Small print helpers shared by the subcommands.

use anyhow::Result;
use lifeagent_core::app::{ChatTurn, TurnNotice};
use serde::Serialize;

/// A `send_message` turn's reply, then each activity on its own bullet line, then each change
/// waiting for approval — what `chat` and `say` print (§11).
pub fn print_turn(turn: &ChatTurn) {
    match turn.notice {
        Some(TurnNotice::EmptyReply) => println!("(the model returned no reply)"),
        Some(TurnNotice::TooManySteps) => println!("(stopped: too many steps)"),
        None => println!("{}", turn.reply),
    }
    for activity in &turn.activities {
        println!("  · {activity}");
    }
    for action in &turn.pending_actions {
        println!(
            "  ? awaiting approval: {} {} (`actions approve {}`)",
            action.tool_name, action.preview, action.id
        );
    }
}

/// Every other subcommand's output: pretty-printed JSON, so results stay both readable and
/// scriptable for a developer harness.
pub fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
