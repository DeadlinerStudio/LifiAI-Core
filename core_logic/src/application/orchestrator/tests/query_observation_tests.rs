    #[tokio::test]
    async fn observation_fallback_uses_tool_tasks_when_model_returns_empty() {
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
                            "timeRangeDays": 7
                        },
                        "reason": "检查已有任务"
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "读取了已有任务并整理结果",
                "tasks": [],
                "habits": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "本轮没有新增长期记忆",
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

        let user_input = "看看有没有重复任务".to_string();
        let initial_result = orchestrator
            .process_message(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "fallback-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
                "read_tasks".to_string(),
                0,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result(
                "fallback-tool-call".to_string(),
                json!({
                    "summary": {
                        "count": 2,
                        "dueSoon24h": 0,
                        "overdue": 0
                    },
                    "tasks": [
                        {
                            "notePreview": "用户提及的会议",
                            "name": "开会",
                            "id": 2,
                            "due": "2026-03-20T15:00:00",
                            "status": "OPEN"
                        },
                        {
                            "notePreview": "",
                            "name": "11111",
                            "id": 1,
                            "due": "2026-03-28T00:22:00",
                            "status": "OPEN"
                        }
                    ]
                })
                .to_string(),
            )
            .await
            .unwrap();

        let tasks = final_result.tasks.expect("expected fallback tasks");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "开会");
        assert_eq!(tasks[0].due_time.as_deref(), Some("2026-03-20 15:00:00"));
        assert_eq!(tasks[1].name, "11111");
    }

    #[tokio::test]
    async fn task_creation_after_dedup_read_tasks_requires_create_task_followup() {
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
                            "keywords": ["穿山甲", "技术方案"],
                            "timeRangeDays": 7
                        },
                        "reason": "先检查是否重复",
                        "executionMode": "AUTO"
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [],
                "chatResponse": "没有重复任务，已帮你创建新任务。",
                "toolCalls": []
            })
            .to_string(),
            json!({
                "tasks": [],
                "toolCalls": [
                    {
                        "tool": "create_task",
                        "args": {
                            "name": "穿山甲需求技术方案写完",
                            "note": "两天内完成"
                        },
                        "reason": "查重为空后继续执行新增任务",
                        "executionMode": "ASK_USER"
                    }
                ]
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

        let user_input = "这两天之内要把穿山甲需求的技术方案写完".to_string();
        let initial_result = orchestrator
            .process_message_core(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected read_tasks call");

        orchestrator
            .register_pending_tool_call(
                "create-followup-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
                "read_tasks".to_string(),
                0,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result_core(
                "create-followup-tool-call".to_string(),
                json!({
                    "summary": {
                        "count": 0,
                        "dueSoon24h": 0,
                        "overdue": 0
                    },
                    "tasks": []
                })
                .to_string(),
            )
            .await
            .unwrap()
            .1;

        let followup_call = final_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .expect("expected follow-up create_task call");
        assert_eq!(followup_call.tool, "create_task");
        assert_eq!(followup_call.agent, Some(AgentRole::TaskAgent));
        assert_eq!(
            followup_call
                .args
                .get("name")
                .and_then(serde_json::Value::as_str),
            Some("穿山甲需求技术方案写完")
        );
    }

    #[tokio::test]
    async fn task_query_returns_chat_response_instead_of_task_proposals() {
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
                            "timeRangeDays": 7
                        },
                        "reason": "需要先请求近期任务上下文"
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [
                    {
                        "name": "开会",
                        "dueTime": "2026-03-20 15:00",
                        "note": "用户提及的会议"
                    },
                    {
                        "name": "准备后天下午3点的开发会议",
                        "dueTime": "2026-03-21 15:00",
                        "note": "需要提前准备会议议程和材料"
                    }
                ],
                "requestedCapabilities": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "用户查看了任务列表",
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

        let user_input = "那我这周有什么任务呀？".to_string();
        let initial_result = orchestrator
            .process_message_core(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "query-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
                "read_tasks".to_string(),
                0,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result_core(
                "query-tool-call".to_string(),
                json!({
                    "tasks": [
                        {
                            "notePreview": "用户提及的会议",
                            "name": "开会",
                            "id": 2,
                            "due": "2026-03-20T15:00:00",
                            "status": "OPEN"
                        },
                        {
                            "notePreview": "",
                            "name": "准备后天下午3点的开发会议",
                            "id": 1,
                            "due": "2026-03-21T15:00:00",
                            "status": "OPEN"
                        }
                    ],
                    "summary": {
                        "count": 2,
                        "dueSoon24h": 0,
                        "overdue": 0
                    }
                })
                .to_string(),
            )
            .await
            .unwrap()
            .1;

        assert_eq!(final_result.primary_intent.as_deref(), Some("Chat"));
        assert_eq!(
            final_result.tasks.as_ref().map(|tasks| tasks.len()),
            Some(0)
        );
        assert_eq!(
            final_result
                .retrieved_tasks
                .as_ref()
                .map(|tasks| tasks.len()),
            Some(2)
        );
        assert!(
            final_result
                .chat_response
                .as_deref()
                .unwrap_or("")
                .contains("2 个相关任务")
        );
    }

    #[tokio::test]
    async fn english_task_query_returns_chat_response_instead_of_task_proposals() {
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
                            "timeRangeDays": 7
                        },
                        "reason": "Need to request recent task context first."
                    }
                ]
            })
            .to_string(),
            json!({
                "tasks": [],
                "chatResponse": "I found 3 tasks for you.",
                "requestedCapabilities": [],
                "toolCalls": []
            })
            .to_string(),
            json!({
                "newMemories": [],
                "userProfile": null,
                "sessionSummary": "User viewed the task list.",
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

        let user_input = "Read my tasks".to_string();
        let initial_result = orchestrator
            .process_message_core(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "english-query-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
                "read_tasks".to_string(),
                0,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result_core(
                "english-query-tool-call".to_string(),
                json!({
                    "tasks": [
                        {
                            "notePreview": "用户提及的会议",
                            "name": "开会",
                            "id": 2,
                            "due": "2026-03-20T15:00:00",
                            "status": "OPEN"
                        },
                        {
                            "notePreview": "需要提前准备会议议程和材料",
                            "name": "准备后天下午3点的开发会议",
                            "id": 1,
                            "due": "2026-03-21T15:00:00",
                            "status": "OPEN"
                        }
                    ],
                    "summary": {
                        "count": 2,
                        "dueSoon24h": 0,
                        "overdue": 0
                    }
                })
                .to_string(),
            )
            .await
            .unwrap()
            .1;

        assert_eq!(final_result.primary_intent.as_deref(), Some("Chat"));
        assert_eq!(
            final_result.tasks.as_ref().map(|tasks| tasks.len()),
            Some(0)
        );
        assert_eq!(
            final_result
                .retrieved_tasks
                .as_ref()
                .map(|tasks| tasks.len()),
            Some(2)
        );
        assert!(
            final_result
                .chat_response
                .as_deref()
                .unwrap_or("")
                .contains("I found 3 tasks")
        );
    }

    #[tokio::test]
    async fn retry_guard_blocks_switching_tool_after_failed_observation() {
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
                            "timeRangeDays": 7
                        },
                        "reason": "先读取任务"
                    }
                ]
            })
            .to_string(),
            json!({
                "toolCalls": [
                    {
                        "tool": "update_deadline",
                        "args": {
                            "taskId": "1",
                            "newDueTime": "2026-03-22 10:00"
                        },
                        "reason": "改用另一个工具"
                    }
                ]
            })
            .to_string(),
        ]));

        let memory_store: Arc<dyn MemoryStore> = Arc::new(MockMemoryStore);
        let registry = Arc::new(SkillRegistry::from_skills(vec![
            ToolMetadata {
                name: "read_tasks".to_string(),
                description: "读取任务".to_string(),
                domain: AgentRole::TaskAgent,
                capabilities: vec![ToolCapability::ReadTaskContext],
                supported_platforms: vec!["ios".to_string()],
                requires_ui_confirmation: false,
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "timeRangeDays": { "type": "integer" }
                    }
                }),
            },
            ToolMetadata {
                name: "update_deadline".to_string(),
                description: "更新任务时间".to_string(),
                domain: AgentRole::TaskAgent,
                capabilities: vec![ToolCapability::UpdateTaskDeadline],
                supported_platforms: vec!["ios".to_string()],
                requires_ui_confirmation: true,
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "taskId": { "type": "string" },
                        "newDueTime": { "type": "string" }
                    },
                    "required": ["taskId", "newDueTime"]
                }),
            },
        ]));
        let orchestrator = Orchestrator::new(
            llm_gateway,
            memory_store,
            registry,
            "ios".to_string(),
            "zh-CN".to_string(),
        );

        let user_input = "看看我的任务".to_string();
        let initial_result = orchestrator
            .process_message_core(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "retry-guard-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::TaskAgent),
                user_input,
                "read_tasks".to_string(),
                0,
            )
            .await;

        let err = orchestrator
            .submit_tool_result_core(
                "retry-guard-tool-call".to_string(),
                json!({
                    "ok": false,
                    "errorCode": "DB_TIMEOUT"
                })
                .to_string(),
            )
            .await
            .expect_err("retry guard should reject tool switching");
        match err {
            AIError::InvalidToolCall(message) => assert!(message.contains("禁止切换工具")),
            other => panic!("unexpected error: {}", other),
        }
    }

    #[tokio::test]
    async fn observation_binding_failure_triggers_one_constrained_regeneration() {
        let llm_gateway: Arc<dyn LlmGateway> = Arc::new(MockLlmGateway::new(vec![
            json!({
                "primaryIntent": "ExtractHabits",
                "routedAgents": ["HabitAgent"]
            })
            .to_string(),
            json!({
                "toolCalls": [
                    {
                        "tool": "read_habits",
                        "args": {
                            "keywords": ["早起"]
                        }
                    }
                ]
            })
            .to_string(),
            json!({
                "chatResponse": "我已经处理好了。",
                "toolCalls": []
            })
            .to_string(),
            json!({
                "habits": [
                    {
                        "name": "早起",
                        "period": "DAILY",
                        "timesPerPeriod": 1,
                        "goalType": "COUNT",
                        "totalTarget": null
                    }
                ],
                "toolCalls": []
            })
            .to_string(),
        ]));

        let memory_store: Arc<dyn MemoryStore> = Arc::new(MockMemoryStore);
        let registry = Arc::new(SkillRegistry::from_skills(vec![ToolMetadata {
            name: "read_habits".to_string(),
            description: "读取习惯".to_string(),
            domain: AgentRole::HabitAgent,
            capabilities: vec![ToolCapability::ReadHabitContext],
            supported_platforms: vec!["ios".to_string()],
            requires_ui_confirmation: false,
            parameters: json!({
                "type": "object",
                "properties": {
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            }),
        }]));
        let orchestrator = Orchestrator::new(
            llm_gateway,
            memory_store,
            registry,
            "ios".to_string(),
            "zh-CN".to_string(),
        );

        let user_input = "看看我的习惯".to_string();
        let initial_result = orchestrator
            .process_message_core(user_input.clone())
            .await
            .unwrap();
        let tool_call = initial_result
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.first())
            .cloned()
            .expect("expected tool call");

        orchestrator
            .register_pending_tool_call(
                "binding-tool-call".to_string(),
                tool_call.agent.unwrap_or(AgentRole::HabitAgent),
                user_input,
                "read_habits".to_string(),
                0,
            )
            .await;

        let final_result = orchestrator
            .submit_tool_result_core(
                "binding-tool-call".to_string(),
                json!({
                    "habits": [
                        {
                            "name": "早起",
                            "id": 2,
                            "period": "DAILY",
                            "timesPerPeriod": 1
                        }
                    ]
                })
                .to_string(),
            )
            .await
            .unwrap()
            .1;

        assert_eq!(
            final_result
                .retrieved_habits
                .as_ref()
                .map(|habits| habits.len()),
            Some(1)
        );
        assert!(
            final_result
                .chat_response
                .as_deref()
                .unwrap_or("")
                .contains("早起")
        );
    }
