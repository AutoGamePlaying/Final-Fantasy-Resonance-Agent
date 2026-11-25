mod common;

use agp_core::RunId;
use agp_game::{CallContext, PROTOCOL_VERSION, serve};
use common::{FakeMod, TestDir, build_merged_fixture_export, module_with_data};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::task::JoinHandle;

struct Client {
    reader: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
}

impl Client {
    async fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.writer
            .write_all(request.to_string().as_bytes())
            .await
            .expect("write request");
        self.writer.write_all(b"\n").await.expect("write newline");
        self.writer.flush().await.expect("flush request");

        let mut line = String::new();
        let bytes = self
            .reader
            .read_line(&mut line)
            .await
            .expect("read response");
        assert_ne!(bytes, 0, "server closed before responding");
        serde_json::from_str(&line).expect("response is JSON")
    }
}

#[tokio::test]
async fn memory_pipe_serves_initialize_tools_data_queries_and_paginated_resources() {
    let bridge_dir = TestDir::new();
    let export_dir = TestDir::new();
    build_merged_fixture_export(export_dir.path());

    let fake_mod = FakeMod::start(bridge_dir.path()).await;
    let module = module_with_data(bridge_dir.path(), export_dir.path());
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(client);
    let (server_reader, server_writer) = tokio::io::split(server);
    let cancellation = CallContext::new(RunId::new()).cancellation;
    let server: JoinHandle<agp_game::Result<()>> =
        tokio::spawn(serve(module, server_reader, server_writer, cancellation));
    let mut client = Client {
        reader: BufReader::new(reader),
        writer,
    };

    let initialized = client
        .request(
            1,
            "initialize",
            json!({
                "protocol_version": PROTOCOL_VERSION,
                "client_name": "ffr-module-test",
                "client_version": "1",
            }),
        )
        .await;
    assert_eq!(initialized["result"]["info"]["game_id"], "ffr");
    assert_eq!(
        initialized["result"]["info"]["module_version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(initialized["result"]["info"]["toolset_version"], "ffr-2");

    let tools = client.request(2, "tools/list", json!({})).await;
    let tool_list = tools["result"]["tools"].as_array().expect("tools is array");
    assert_eq!(tool_list.len(), 17);
    let names: Vec<&str> = tool_list
        .iter()
        .map(|t| t["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(
        names,
        vec![
            "game_status",
            "bridge_ping",
            "read_game_state",
            "read_save_slots",
            "read_inventory",
            "read_event_flags",
            "read_input_state",
            "query_nav_path",
            "read_chapter",
            "read_map",
            "read_enemy",
            "read_battle_group",
            "read_skill",
            "read_item",
            "read_quest",
            "read_shop",
            "search_data",
        ]
    );
    for tool in &tool_list[..8] {
        assert_eq!(tool["category"], "status");
        assert_eq!(tool["side_effects"], false);
    }
    for tool in &tool_list[8..] {
        assert_eq!(tool["category"], "data");
        assert_eq!(tool["side_effects"], false);
    }

    let status = client
        .request(
            3,
            "tools/call",
            json!({"name": "game_status", "args": {}, "run_id": RunId::new()}),
        )
        .await;
    assert_eq!(
        status["result"]["output"]["content"][0]["value"]["bridge"]["ok"],
        true
    );
    assert_eq!(status["result"]["output"]["is_error"], false);

    let enemy = client
        .request(
            4,
            "tools/call",
            json!({
                "name": "read_enemy",
                "args": { "name": "熔核守护者" },
                "run_id": RunId::new(),
            }),
        )
        .await;
    assert_eq!(enemy["result"]["output"]["is_error"], false);
    assert_eq!(enemy["result"]["output"]["content"][0]["type"], "text");
    assert!(
        enemy["result"]["output"]["content"][0]["text"]
            .as_str()
            .expect("enemy summary")
            .contains("熔核守护者")
    );
    assert_eq!(enemy["result"]["output"]["content"][1]["type"], "json");
    assert_eq!(
        enemy["result"]["output"]["content"][1]["value"]["unit_id"],
        9201
    );
    assert_eq!(
        enemy["result"]["output"]["content"][1]["value"]["is_boss"],
        true
    );

    let search = client
        .request(
            5,
            "tools/call",
            json!({
                "name": "search_data",
                "args": { "query": "微光" },
                "run_id": RunId::new(),
            }),
        )
        .await;
    assert_eq!(search["result"]["output"]["is_error"], false);
    assert!(
        search["result"]["output"]["content"][1]["value"]["total_matches"]
            .as_u64()
            .expect("total_matches")
            >= 2
    );

    let resources = client.request(6, "resources/list", json!({})).await;
    let res_list = resources["result"]["resources"]
        .as_array()
        .expect("resources is array");
    let uris: Vec<&str> = res_list
        .iter()
        .map(|r| r["uri"].as_str().expect("uri"))
        .collect();
    assert_eq!(
        uris,
        vec![
            "ffr://chapters",
            "ffr://maps",
            "ffr://enemies",
            "ffr://skills",
        ]
    );
    for res in res_list {
        assert_eq!(res["mime_type"], "application/json");
        assert!(res["size_hint"].as_u64().is_some_and(|n| n > 0));
    }

    // Read ffr://enemies in small pages over JSON-RPC and reconstruct valid JSON.
    let mut assembled = String::new();
    let mut offset = 0_u64;
    let mut req_id = 7_u64;
    loop {
        let resp = client
            .request(
                req_id,
                "resources/read",
                json!({
                    "uri": "ffr://enemies",
                    "range": { "offset": offset, "length": 80 },
                }),
            )
            .await;
        req_id += 1;
        let page = &resp["result"]["page"];
        assembled.push_str(page["content"]["text"].as_str().expect("page text"));
        match page["next_offset"].as_u64() {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    let parsed: Value = serde_json::from_str(&assembled).expect("reconstructed JSON is valid");
    assert_eq!(parsed["enemies"].as_array().map(Vec::len), Some(3));

    client.writer.shutdown().await.expect("close client");
    server
        .await
        .expect("join server")
        .expect("server accepts EOF");
    fake_mod.stop().await;
}
