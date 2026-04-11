    #[tokio::test]
    async fn legacy_action_is_still_normalized_into_registered_read_tool() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "actions": [
                    {
                        "action": "READ_CONTEXT",
                        "args": {
                            "timeRangeDays": 7,
                            "keywords": ["本周"]
                        },
                        "reason": "回答前先读取本地任务上下文"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
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
            .process_message("那我这周有什么任务呀？".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected normalized tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(7));
        assert_eq!(tool_call.args["keywords"], json!(["本周"]));
        assert_eq!(tool_call.agent, Some(AgentRole::TaskAgent));
    }

    #[tokio::test]
    async fn requested_capability_is_normalized_into_registered_read_tool() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 7,
                            "keywords": ["本周"]
                        },
                        "reason": "回答前先请求任务上下文"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
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
            .process_message("那我这周有什么任务呀？".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected normalized tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(7));
        assert_eq!(tool_call.args["keywords"], json!(["本周"]));
        assert_eq!(tool_call.agent, Some(AgentRole::TaskAgent));
    }

    #[tokio::test]
    async fn requested_capability_defaults_to_auto_for_single_non_confirm_tool() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 5
                        },
                        "reason": "读取任务上下文"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
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
            .process_message("看看我的任务".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(5));
    }

    #[tokio::test]
    async fn low_confidence_tool_call_is_gated_to_ask_user() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 7,
                            "dueTime": "tomorrow noon"
                        },
                        "reason": "尝试读取上下文"
                    }
                ]
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "本轮未写入记忆",
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
            .process_message("帮我处理一下".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");
        assert_eq!(
            tool_call.execution_mode,
            Some(crate::domain::models::CapabilityExecutionMode::AskUser)
        );
        assert!(
            tool_call
                .reason
                .as_deref()
                .unwrap_or("")
                .contains("confidence_gate")
        );
    }

    #[tokio::test]
    async fn requested_capability_with_never_does_not_emit_tool_call() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "chatResponse": "我暂时不读取本地任务，先根据已有信息回答。",
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "executionMode": "NEVER",
                        "args": {
                            "timeRangeDays": 7
                        },
                        "reason": "本轮禁止读取工具"
                    }
                ],
                "tasks": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "未读取任务上下文",
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
            .process_message("看看我的任务".to_string())
            .await
            .unwrap();

        assert!(
            result
                .tool_calls
                .as_ref()
                .map(|calls| calls.is_empty())
                .unwrap_or(true)
        );
        assert_eq!(
            result.chat_response.as_deref(),
            Some("我暂时不读取本地任务，先根据已有信息回答。")
        );
    }

    #[tokio::test]
    async fn requested_capabilities_take_precedence_over_legacy_actions() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 14
                        },
                        "reason": "优先使用新协议"
                    }
                ],
                "actions": [
                    {
                        "action": "READ_CONTEXT",
                        "args": {
                            "timeRangeDays": 3
                        },
                        "reason": "旧协议不应覆盖新协议"
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
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
            .process_message("看看我的任务".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected normalized tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(14));
    }

    #[tokio::test]
    async fn requested_capabilities_take_precedence_over_hallucinated_tool_calls_and_actions() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "requestedCapabilities": [
                    {
                        "capability": "READ_TASK_CONTEXT",
                        "args": {
                            "timeRangeDays": 10
                        },
                        "reason": "按新协议读取任务上下文"
                    }
                ],
                "toolCalls": [
                    {
                        "tool": "not_exists_tool",
                        "args": {},
                        "reason": "幻觉工具"
                    }
                ],
                "actions": [
                    {
                        "action": "READ_CONTEXT",
                        "args": {
                            "timeRangeDays": 1
                        },
                        "reason": "旧协议不应生效"
                    }
                ]
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "准备读取任务上下文",
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
            .process_message("看看我的任务".to_string())
            .await
            .unwrap();

        let tool_call = result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected normalized tool call");

        assert_eq!(tool_call.tool, "read_tasks");
        assert_eq!(tool_call.args["timeRangeDays"], json!(10));
    }

    #[tokio::test]
    async fn invalid_tool_call_args_are_downgraded_to_safe_chat() {
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
                            "timeRangeDays": "seven"
                        },
                        "reason": "参数故意写错"
                    }
                ]
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": null,
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
            .process_message("帮我看看下周任务".to_string())
            .await
            .expect("invalid tool call should be downgraded");

        assert!(
            result
                .tool_calls
                .as_ref()
                .map(|calls| calls.is_empty())
                .unwrap_or(true)
        );
        assert_eq!(result.primary_intent.as_deref(), Some("Chat"));
        assert!(
            result
                .chat_response
                .as_deref()
                .unwrap_or("")
                .contains("不执行本地任务工具")
        );
    }

    #[tokio::test]
    async fn unknown_action_is_downgraded_to_safe_chat() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractTasks",
                "routedAgents": ["TaskAgent"]
            })
            .to_string(),
            json!({
                "actions": [
                    {
                        "action": "READ_TASK_LIST",
                        "args": {
                            "timeRangeDays": 7
                        }
                    }
                ]
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": null,
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
            .process_message("帮我看看这周任务".to_string())
            .await
            .expect("invalid action should be downgraded");

        assert!(
            result
                .tool_calls
                .as_ref()
                .map(|calls| calls.is_empty())
                .unwrap_or(true)
        );
        assert_eq!(result.primary_intent.as_deref(), Some("Chat"));
        assert!(
            result
                .chat_response
                .as_deref()
                .unwrap_or("")
                .contains("不执行本地任务工具")
        );
    }
