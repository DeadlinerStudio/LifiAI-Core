    #[tokio::test]
    async fn process_message_and_submit_tool_result_form_a_local_mock_loop() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "toolCalls": [
                    {
                        "tool": "read_tasks",
                        "args": {
                            "timeRangeDays": 7,
                            "keywords": ["Rust", "冒烟测试"]
                        },
                        "reason": "需要检查是否已有重复任务"
                    }
                ]
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备执行任务去重工具",
                "tasks": [],
                "habits": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "tasks": [
                    {
                        "name": "写 Rust 冒烟测试",
                        "dueTime": "2026-03-19 10:00",
                        "note": "避免重复后确认创建"
                    }
                ]
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "完成了一次任务去重检查",
                "tasks": [],
                "habits": [],
                "toolCalls": []
            })
            .to_string(),
        ]));

        let memory_store: Arc<dyn MemoryStore> = Arc::new(MockMemoryStore);
        let orchestrator = Orchestrator::new(
            llm_gateway,
            memory_store,
            build_test_registry(),
            "ios".to_string(),
            "zh-CN".to_string(),
        );

        let user_input = "帮我记一下明天上午十点写 Rust 冒烟测试".to_string();
        let initial_result = orchestrator
            .process_message(user_input.clone())
            .await
            .unwrap();

        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected one tool call");
        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.agent, Some(AgentRole::TaskAgent));

        let tool_call_id = "tool-call-1".to_string();
        orchestrator
            .register_pending_tool_call(
                tool_call_id.clone(),
                AgentRole::TaskAgent,
                user_input,
                "read_tasks".to_string(),
                0,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result(
                tool_call_id,
                json!({
                    "tasks": []
                })
                .to_string(),
            )
            .await
            .unwrap();

        let tasks = final_result.tasks.expect("expected extracted tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "写 Rust 冒烟测试");
        assert_eq!(tasks[0].due_time.as_deref(), Some("2026-03-19 10:00"));
    }
