    #[tokio::test]
    async fn final_memory_synthesizer_adds_memories_to_merged_result() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "tasks": [
                    {
                        "name": "写 Rust 冒烟测试",
                        "dueTime": "2026-03-19 10:00",
                        "note": "明天上午十点"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": ["用户最近在推进 Rust 测试相关开发"],
                "userProfile": "用户当前在进行 Rust 工程开发",
                "sessionSummary": "记录了一条 Rust 冒烟测试任务",
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

        let result = orchestrator
            .process_message("帮我记一下明天上午十点写 Rust 冒烟测试".to_string())
            .await
            .unwrap();

        assert_eq!(result.tasks.as_ref().map(|tasks| tasks.len()), Some(1));
        assert_eq!(
            result.new_memories,
            Some(vec!["用户最近在推进 Rust 测试相关开发".to_string()])
        );
        assert_eq!(
            result.user_profile.as_deref(),
            Some("用户当前在进行 Rust 工程开发")
        );
        assert_eq!(
            result.session_summary.as_deref(),
            Some("记录了一条 Rust 冒烟测试任务")
        );
    }
