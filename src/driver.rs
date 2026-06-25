use crate::{
    checker::LeanCheckResult,
    config::Config,
    context::{Context, display_relative_path, render_with_line_numbers},
    templates::{
        TemplateEngine, add_import, read_file, recall_directory_structure, system_prompt,
        update_lemma, update_proof, user_prompt, write_scratch_file,
    },
};
use anyhow::{Context as _, bail, ensure};
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCallChunk,
    ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessage,
    ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessage, ChatCompletionRequestSystemMessageContent,
    ChatCompletionRequestToolMessage, ChatCompletionRequestToolMessageContent,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent, ChatCompletionTool,
    ChatCompletionTools, CreateChatCompletionRequest, FunctionCall, FunctionObject,
    ReasoningEffort,
};
use clap::Parser;
use colored::Colorize;
use futures::StreamExt;
use pollster::FutureExt;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Parser)]
pub struct Args {
    pub file: String,
    #[clap(long)]
    pub theorem: String,
    #[clap(long)]
    pub model: String,
}

pub fn run_driver(config: &Config, args: &Args) -> anyhow::Result<()> {
    let mut context = Context::new(Path::new("."), &args.file, &args.theorem)?;

    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _tokio_guard = tokio_runtime.enter();

    let model = config
        .models
        .get(&args.model)
        .context("model not found in config")?;
    let provider = &config.providers[&model.provider];

    let mut api_config = async_openai::config::OpenAIConfig::new().with_api_base(&provider.url);
    if let Some(api_key) = &provider.api_key {
        api_config = api_config.with_api_key(api_key.clone());
    }

    let api = async_openai::Client::with_config(api_config);
    let api = api.chat();

    let template_engine = TemplateEngine::load();

    let tools = [
        ("add_import", schema_for!(AddImportParams)),
        ("read_file", schema_for!(ReadFileParams)),
        (
            "recall_directory_structure",
            schema_for!(RecallDirectoryStructureParams),
        ),
        ("update_lemma", schema_for!(UpdateLemmaParams)),
        ("update_proof", schema_for!(UpdateProofParams)),
        ("write_scratch_file", schema_for!(WriteScratchFileParams)),
    ]
    .into_iter()
    .map(|(name, parameters)| {
        ChatCompletionTools::Function(ChatCompletionTool {
            function: FunctionObject {
                name: name.to_owned(),
                description: Some(
                    template_engine.render(&format!("tools/{name}/description.j2"), ()),
                ),
                parameters: Some(parameters.to_value()),
                strict: Some(true),
            },
        })
    })
    .collect::<Vec<_>>();

    let build_request = |messages: &[HistoryMessage]| -> anyhow::Result<serde_json::Value> {
        let request = CreateChatCompletionRequest {
            model: model.api_name.clone(),
            reasoning_effort: Some(ReasoningEffort::Xhigh), // TODO don't hardcode
            max_completion_tokens: Some(model.max_output_tokens),
            tools: Some(tools.clone()),
            parallel_tool_calls: Some(true),
            ..Default::default()
        };
        let mut request = serde_json::to_value(request)?;
        request["messages"] = serde_json::to_value(messages)?;
        request["stream"] = serde_json::Value::Bool(true);
        Ok(request)
    };

    let file_tree =
        crate::pretty_file_tree::generate_file_tree(context.dir(), |file| file.ends_with(".lean"))?;
    tracing::debug!("File tree: {file_tree}");

    let system_prompt = template_engine.render(
        "system.j2",
        system_prompt::Data {
            project_file_tree: file_tree.clone(),
        },
    );
    let user_prompt = template_engine.render(
        "user.j2",
        user_prompt::Data {
            theorem_file_path: display_relative_path(&args.file),
            theorem_file_contents: context
                .read_file(&args.file)?
                .context("couldn't read target theorem file")?,
            theorem_name: args.theorem.clone(),
        },
    );

    let mut messages = vec![
        HistoryMessage::Standard(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(system_prompt),
                name: None,
            },
        )),
        HistoryMessage::Standard(ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(user_prompt),
                name: None,
            },
        )),
    ];

    let mut output_printer = LlmOutputPrinter::default();

    while !context.theorem_file_editor_mut().has_theorem_proof() {
        // Stream the response, pushing reasoning and message content into the printer as soon as
        // each chunk arrives. The accumulated message is reconstructed from the deltas so it can be
        // appended to the history and its tool calls executed once the turn ends.
        let mut stream = api
            .create_stream_byot::<_, ReasoningChatCompletionChunk>(build_request(&messages)?)
            .block_on()?;

        let mut content = String::new();
        let mut reasoning_content = String::new();
        let mut refused = false;
        let mut tool_calls: Vec<ToolCallAccumulator> = Vec::new();

        while let Some(chunk) = stream.next().block_on() {
            let chunk = chunk?;
            let Some(choice) = chunk.choices.into_iter().next() else {
                // skip empty chunks
                continue;
            };
            let delta = choice.delta;

            if delta.refusal.is_some() {
                refused = true;
            }
            if let Some(reasoning) = &delta.reasoning_content {
                reasoning_content.push_str(reasoning);
                output_printer.push(reasoning);
            }
            if let Some(chunk_content) = &delta.content {
                content.push_str(chunk_content);
                output_printer.push(chunk_content);
            }
            for tool_call_chunk in delta.tool_calls.into_iter().flatten() {
                let index = tool_call_chunk.index as usize;
                if index >= tool_calls.len() {
                    tool_calls.resize_with(index + 1, ToolCallAccumulator::default);
                }
                let accumulator = &mut tool_calls[index];
                if let Some(id) = tool_call_chunk.id {
                    accumulator.id.push_str(&id);
                }
                if let Some(function) = tool_call_chunk.function {
                    if let Some(name) = function.name {
                        accumulator.name.push_str(&name);
                    }
                    if let Some(arguments) = function.arguments {
                        accumulator.arguments.push_str(&arguments);
                    }
                }

                output_printer.flush();
            }
        }

        // Turn over.
        output_printer.flush();

        ensure!(!refused);

        let tool_calls: Vec<ChatCompletionMessageToolCalls> = tool_calls
            .into_iter()
            .map(|accumulator| {
                ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                    id: accumulator.id,
                    function: FunctionCall {
                        name: accumulator.name,
                        arguments: accumulator.arguments,
                    },
                })
            })
            .collect();

        let reasoning_content = (!reasoning_content.is_empty()).then_some(reasoning_content);

        messages.push(HistoryMessage::Assistant(AssistantHistoryMessage {
            role: AssistantRole::Assistant,
            message: ChatCompletionRequestAssistantMessage {
                content: (!content.is_empty())
                    .then_some(content)
                    .map(ChatCompletionRequestAssistantMessageContent::Text),
                tool_calls: (!tool_calls.is_empty()).then(|| tool_calls.clone()),
                ..Default::default()
            },
            reasoning_content,
        }));

        for tool_call in &tool_calls {
            let ChatCompletionMessageToolCalls::Function(tool_call) = tool_call else {
                bail!("wrong custom tool call received from api")
            };

            tracing::info!("Model invoked tool: {}", tool_call.function.name);

            let response = match tool_call.function.name.as_str().trim() {
                "add_import" => {
                    let params: AddImportParams =
                        serde_json::from_str(&tool_call.function.arguments)?;

                    tracing::info!("Attempting to add import of '{}'", params.import_file);

                    match context
                        .theorem_file_editor_mut()
                        .add_import(&params.import_file)?
                    {
                        LeanCheckResult::Success => template_engine
                            .render("tools/add_import/success.j2", add_import::SuccessData {}),
                        LeanCheckResult::Failure { output, .. } => {
                            tracing::info!("Import add led to failed compile");
                            template_engine.render(
                                "tools/add_import/failure.j2",
                                add_import::FailureData {
                                    failure_reason: format!(
                                        "lean compilation failed with errors: {output}"
                                    ),
                                },
                            )
                        }
                    }
                }
                "read_file" => {
                    let params: ReadFileParams =
                        serde_json::from_str(&tool_call.function.arguments)?;
                    tracing::info!("Reading file '{}'", params.path);
                    match context.read_file(&params.path)? {
                        Some(file_contents) => template_engine.render(
                            "tools/read_file/success.j2",
                            read_file::SuccessData {
                                file_path: params.path,
                                file_contents,
                            },
                        ),
                        None => {
                            tracing::info!("Model attempted to read nonexistent file");
                            template_engine.render(
                                "tools/read_file/failure.j2",
                                read_file::FailureData {
                                    file_path: params.path,
                                },
                            )
                        }
                    }
                }
                "recall_directory_structure" => template_engine.render(
                    "tools/recall_directory_structure/success.j2",
                    recall_directory_structure::SuccessData {
                        project_file_tree: file_tree.clone(),
                    },
                ),
                "update_lemma" => {
                    let params: UpdateLemmaParams =
                        serde_json::from_str(&tool_call.function.arguments)?;
                    tracing::info!("Updating lemma '{}'", params.lemma_name);
                    match context
                        .theorem_file_editor_mut()
                        .update_lemma(&params.lemma_name, &params.lemma_source_code)?
                    {
                        LeanCheckResult::Success => template_engine.render(
                            "tools/update_lemma/success.j2",
                            update_lemma::SuccessData {
                                lemma_name: params.lemma_name,
                            },
                        ),
                        LeanCheckResult::Failure {
                            output,
                            source_code,
                        } => {
                            tracing::info!("Lemma '{}' failed to check", params.lemma_name);
                            template_engine.render(
                                "tools/update_lemma/failure.j2",
                                update_lemma::FailureData {
                                    lemma_name: params.lemma_name,
                                    failure_reason: format!(
                                        "lean compile failed with errors: {output}"
                                    ),
                                    failed_file_contents: render_with_line_numbers(&source_code),
                                },
                            )
                        }
                    }
                }
                "update_proof" => {
                    let params: UpdateProofParams =
                        serde_json::from_str(&tool_call.function.arguments)?;
                    match context
                        .theorem_file_editor_mut()
                        .update_theorem_proof(&params.proof_source_code)?
                    {
                        LeanCheckResult::Success => {
                            tracing::info!("Proof succeeded");
                            template_engine.render(
                                "tools/update_proof/success.j2",
                                update_proof::SuccessData {},
                            )
                        }
                        LeanCheckResult::Failure {
                            output,
                            source_code,
                        } => {
                            tracing::info!("Proof failed to check");
                            println!("{}", render_with_line_numbers(&source_code));
                            template_engine.render(
                                "tools/update_proof/failure.j2",
                                update_proof::FailureData {
                                    failure_reason: format!(
                                        "lean compile failed with errors: {output}"
                                    ),
                                    failed_file_contents: render_with_line_numbers(&source_code),
                                },
                            )
                        }
                    }
                }
                "write_scratch_file" => {
                    let params: WriteScratchFileParams =
                        serde_json::from_str(&tool_call.function.arguments)?;
                    let output = match context.check_scratch_file(&params.source_code)? {
                        LeanCheckResult::Success => "Successfully compiled!".to_owned(),
                        LeanCheckResult::Failure { output, .. } => {
                            format!("lean compile failed with errors: {output}")
                        }
                    };
                    template_engine.render(
                        "tools/write_scratch_file/output.j2",
                        write_scratch_file::SuccessData {
                            lean_output: output,
                        },
                    )
                }
                t => bail!("model tried to use nonexistent tool: {t:?}"),
            };
            messages.push(HistoryMessage::Standard(
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: ChatCompletionRequestToolMessageContent::Text(response),
                    tool_call_id: tool_call.id.clone(),
                }),
            ));
        }

        if tool_calls.is_empty() {
            tracing::info!("Model appears to have given up");
            break;
        }
    }

    Ok(())
}

#[derive(Default)]
struct LlmOutputPrinter {
    buffer: Vec<char>,
}

impl LlmOutputPrinter {
    const APPROX_MAX_LINE_WIDTH: usize = 128;

    pub fn push(&mut self, content: &str) {
        for (i, line) in content.lines().enumerate() {
            if i > 0 {
                self.flush();
            }

            self.buffer.extend(line.chars());
            self.emit_full_lines();
        }
    }

    fn emit_full_lines(&mut self) {
        while self.buffer.len() > Self::APPROX_MAX_LINE_WIDTH {
            let line_break = self
                .buffer
                .iter()
                .rposition(|&c| c.is_ascii_whitespace())
                .unwrap_or(Self::APPROX_MAX_LINE_WIDTH);

            self.flush_line(&self.buffer[..line_break]);
            self.buffer = self.buffer[line_break + 1..].to_vec();
        }
    }

    pub fn flush(&mut self) {
        self.emit_full_lines();
        if !self.buffer.is_empty() {
            self.flush_line(&self.buffer);
        }
        self.buffer.clear();
    }

    fn flush_line(&self, line: &[char]) {
        println!("> {}", String::from_iter(line.iter().copied()).dimmed());
    }
}

// Have to override async-openai with custom types to support reasoning_content since async-openai
// won't add it

#[derive(Serialize, Clone)]
#[serde(untagged)]
enum HistoryMessage {
    Standard(ChatCompletionRequestMessage),
    Assistant(AssistantHistoryMessage),
}

#[derive(Serialize, Clone)]
struct AssistantHistoryMessage {
    role: AssistantRole,
    #[serde(flatten)]
    message: ChatCompletionRequestAssistantMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "lowercase")]
enum AssistantRole {
    Assistant,
}

#[derive(Deserialize)]
struct ReasoningChatCompletionChunk {
    #[serde(default)]
    choices: Vec<ReasoningChatChoiceChunk>,
}

#[derive(Deserialize)]
struct ReasoningChatChoiceChunk {
    delta: ReasoningResponseDelta,
}

#[derive(Deserialize)]
struct ReasoningResponseDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatCompletionMessageToolCallChunk>>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Default)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

//
// === Tools ===
//

#[derive(Deserialize, JsonSchema, Debug)]
struct AddImportParams {
    #[schemars(
        description = "Name of the target to import, as Lean path syntax, e.g. Path.To.File"
    )]
    import_file: String,
}

#[derive(Deserialize, JsonSchema, Debug)]
struct ReadFileParams {
    #[schemars(
        description = "Path to the file to read. Must start with a '/'. Root corresponds to the project root."
    )]
    path: String,
}

#[derive(Deserialize, JsonSchema, Debug)]
struct RecallDirectoryStructureParams {}

#[derive(Deserialize, JsonSchema, Debug)]
struct UpdateLemmaParams {
    #[schemars(
        description = "Name of the lemma, which must be the same as the lemma identifier in the lemma_source_code."
    )]
    lemma_name: String,
    #[schemars(
        description = "Code that implements the lemma, including both the declaration `lemma ...`) and its body to complete the proof."
    )]
    lemma_source_code: String,
}

#[derive(Deserialize, JsonSchema, Debug)]
struct UpdateProofParams {
    #[schemars(description = "Code that will replace the `sorry` line in the proof.")]
    proof_source_code: String,
}

#[derive(Deserialize, JsonSchema, Debug)]
struct WriteScratchFileParams {
    #[schemars(description = "Full code to write into the scratch file.")]
    source_code: String,
}
