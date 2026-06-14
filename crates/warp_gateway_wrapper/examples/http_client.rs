use reqwest::Client;
use serde_json::json;
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let base_url = "http://127.0.0.1:8080";
    
    // 1. Spawn an agent
    println!("Spawning agent...");
    let spawn_request = json!({
        "prompt": "Write a hello world program in Rust",
        "config": {
            "model": "gpt-4",
            "temperature": 0.7
        }
    });
    
    let spawn_response = client
        .post(&format!("{}/agent/run", base_url))
        .json(&spawn_request)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    
    println!("Spawn response: {}", serde_json::to_string_pretty(&spawn_response)?);
    
    let task_id = spawn_response["task_id"].as_str().unwrap();
    let run_id = spawn_response["run_id"].as_str().unwrap();
    
    println!("\nTask ID: {}", task_id);
    println!("Run ID: {}", run_id);
    
    // 2. Subscribe to event stream
    println!("\nSubscribing to event stream...");
    let stream_url = format!("{}/agent/stream/{}", base_url, task_id);
    
    let mut event_source = reqwest::get(&stream_url)
        .await?
        .bytes_stream();
    
    println!("Listening for events (Ctrl+C to stop)...\n");
    
    while let Some(chunk) = event_source.next().await {
        match chunk {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                if text.starts_with("data: ") {
                    let json_str = text.strip_prefix("data: ").unwrap().trim();
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(json_str) {
                        println!("Event: {}", serde_json::to_string_pretty(&event)?);
                    }
                }
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    
    // 3. Check task status
    println!("\nChecking task status...");
    let status_response = client
        .get(&format!("{}/agent/task/{}", base_url, task_id))
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    
    println!("Status: {}", serde_json::to_string_pretty(&status_response)?);
    
    Ok(())
}
