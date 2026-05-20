#![forbid(unsafe_code)]

use deepseek_coder_agent_core::AGENT_METADATA;

fn main() {
    println!(
        "{} workspace initialized; local state directory: {}",
        AGENT_METADATA.name, AGENT_METADATA.state_dir
    );
}
