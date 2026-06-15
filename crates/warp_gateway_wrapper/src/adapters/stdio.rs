use crate::gateway::GatewayEngine;
use crate::protocol::GatewayRequest;
use std::io::{self, BufRead, Write};

pub struct StdioAdapter {
    engine: GatewayEngine,
}

impl StdioAdapter {
    pub fn new(engine: GatewayEngine) -> Self {
        Self { engine }
    }

    pub async fn run(&self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();

        for line in stdin.lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let request: GatewayRequest = match serde_json::from_str(&line) {
                Ok(req) => req,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to parse stdio request");
                    continue;
                }
            };

            let response = self.engine.handle_request(request).await;

            let response_json = serde_json::to_string(&response)?;
            writeln!(stdout, "{}", response_json)?;
            stdout.flush()?;
        }

        Ok(())
    }
}
