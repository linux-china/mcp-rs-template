mod mcp;

use crate::mcp::prompts::{prompts_get, prompts_list};
use crate::mcp::resources::{resource_read, resources_list};
use crate::mcp::tools::register_tools;
use crate::mcp::types::{
    CancelledNotification, JsonRpcError, JsonRpcResponse, ToolCallRequestParams,
};
use crate::mcp::utilities::*;
use clap::Parser;
use rpc_router::{Error, Handler, Request, Router, RouterBuilder};
use serde_json::{json, Value};
use signal_hook::consts::SIGTERM;
use signal_hook::{consts::SIGINT, iterator::Signals};
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::thread;

fn build_rpc_router() -> Router {
    let builder = RouterBuilder::default()
        // append resources here
        .append_dyn("initialize", initialize.into_dyn())
        .append_dyn("ping", ping.into_dyn())
        .append_dyn("logging/setLevel", logging_set_level.into_dyn())
        .append_dyn("roots/list", roots_list.into_dyn())
        .append_dyn("prompts/list", prompts_list.into_dyn())
        .append_dyn("prompts/get", prompts_get.into_dyn())
        .append_dyn("resources/list", resources_list.into_dyn())
        .append_dyn("resources/read", resource_read.into_dyn());
    let builder = register_tools(builder);
    builder.build()
}

#[tokio::main]
async fn main() {
    // clap args parser
    let args = Args::parse();
    if !args.mcp {
        display_info(&args);
        return;
    }
    // signal handling to exit cli
    let mut signals = Signals::new([SIGTERM, SIGINT]).unwrap();
    thread::spawn(move || {
        for _sig in signals.forever() {
            graceful_shutdown();
            std::process::exit(0);
        }
    });
    // process json-rpc from MCP client
    let router = build_rpc_router();
    let mut line = String::new();
    let input = io::stdin();
    let mut logging_file = OpenOptions::new()
        .write(true)
        .append(true)
        .open("/tmp/mcp.jsonl")
        .unwrap();
    while input.read_line(&mut line).unwrap() != 0 {
        let line = std::mem::take(&mut line);
        writeln!(logging_file, "{}", line).unwrap();
        if !line.is_empty() {
            if let Ok(json_value) = serde_json::from_str::<Value>(&line) {
                // notifications, no response required
                if json_value.is_object() && json_value.get("id").is_none() {
                    if let Some(method) = json_value.get("method") {
                        if method == "notifications/initialized" {
                            notifications_initialized();
                        } else if method == "notifications/cancelled" {
                            let params_value = json_value.get("params").unwrap();
                            let cancel_params: CancelledNotification =
                                serde_json::from_value(params_value.clone()).unwrap();
                            notifications_cancelled(cancel_params);
                        }
                    }
                } else if let Ok(mut rpc_request) = Request::from_value(json_value) {
                    // normal json-rpc message, and response expected
                    let id = rpc_request.id.clone();
                    if rpc_request.method == "tools/call" {
                        let params = serde_json::from_value::<ToolCallRequestParams>(
                            rpc_request.params.unwrap(),
                        )
                        .unwrap();
                        rpc_request = Request {
                            id: id.clone(),
                            method: params.name,
                            params: params.arguments,
                        }
                    }
                    match router.call(rpc_request).await {
                        Ok(call_response) => {
                            if !call_response.value.is_null() {
                                let response =
                                    JsonRpcResponse::new(id, call_response.value.clone());
                                let response_json = serde_json::to_string(&response).unwrap();
                                writeln!(logging_file, "{}\n", response_json).unwrap();
                                println!("{}", response_json);
                            }
                        }
                        Err(error) => match &error.error {
                            // error from json-rpc call
                            Error::Handler(handler) => {
                                if let Some(error_value) = handler.get::<Value>() {
                                    let json_error = json!({
                                        "jsonrpc": "2.0",
                                        "error": error_value,
                                        "id": id
                                    });
                                    let response = serde_json::to_string(&json_error).unwrap();
                                    writeln!(logging_file, "{}\n", response).unwrap();
                                    println!("{}", response);
                                }
                            }
                            _ => {
                                let json_error = JsonRpcError::new(id, -1, "Invalid json-rpc call");
                                let response = serde_json::to_string(&json_error).unwrap();
                                writeln!(logging_file, "{}\n", response).unwrap();
                                println!("{}", response);
                            }
                        },
                    }
                }
            }
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// list resources
    #[arg(long, default_value = "false")]
    resources: bool,
    /// list prompts
    #[arg(long, default_value = "false")]
    prompts: bool,
    /// list tools
    #[arg(long, default_value = "false")]
    tools: bool,
    /// start MCP server
    #[arg(long, default_value = "false")]
    mcp: bool,
}

impl Args {
    fn is_args_available(&self) -> bool {
        self.prompts || self.resources || self.tools
    }
}

fn display_info(args: &Args) {
    if !args.is_args_available() {
        println!("Please use --help to see available options");
        return;
    }
    if args.prompts {
        println!(
            r#"prompts:
- current_time: get current time in city
"#
        );
    }
    if args.resources {
        println!(
            r#"resources:
- sqlite: file:///path/to/sqlite.db
"#
        );
    }
    if args.tools {
        println!(
            r#"tools:
- get_current_time_in_city: get current time in city
"#
        );
    }
}