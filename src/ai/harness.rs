use crate::ai::runner::run_cli_turn;
use crate::ai::{AiToolStep, AiUiEvent, CliProviderId};
use crate::models::ExecutionTarget;
use crate::runner::{
    ModelSchemaInfo, SchemaIntrospectRequest, introspect_laravel_schema, lint_php_syntax_with_bin,
};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

const TURN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_TOOL_ROUNDS: usize = 3;

static CODE_BLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)```(?:php|PHP)?\s*\n(.*?)```").unwrap());

pub struct AiHarnessJobInput {
    pub msg_id: u64,
    pub provider: CliProviderId,
    pub model: String,
    pub effort: Option<String>,
    pub user_prompt: String,
    pub prior_turns: Vec<(String, String)>,
    pub project_path: String,
    pub target: ExecutionTarget,
    pub php_binary: String,
    pub laravel_version: String,
    pub php_version: String,
    pub indexed_models: Vec<String>,
    pub editor_code: Option<String>,
    pub last_exception: Option<String>,
    pub allow_ssh_schema: bool,
    pub known_schemas: HashMap<String, ModelSchemaInfo>,
    pub known_tables: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedToolCall {
    InspectModels(Vec<String>),
    InspectTables(Vec<String>),
    ListTables,
    ReadModelSource(String),
}

pub fn execute_ai_harness_job(
    input: AiHarnessJobInput,
    cancel: &AtomicBool,
    tx: &Sender<AiUiEvent>,
) {
    let can_introspect_db = !input.target.is_ssh() || input.allow_ssh_schema;
    let mut schemas = input.known_schemas.clone();
    let mut newly_cached_models: Vec<ModelSchemaInfo> = Vec::new();
    let mut all_tables = input.known_tables.clone();
    let mut step_counter = 0usize;

    // Smart Tier-1 Pre-Match: detect model names mentioned in the user's prompt (or @Model)
    // and pre-fetch up to 4 uncached Eloquent model schemas before Turn 1 so simple queries
    // like "How many user register at yesterday" often finish in a single round-trip!
    if can_introspect_db && !input.indexed_models.is_empty() {
        let matched = match_prompt_models(&input.user_prompt, &input.indexed_models);
        let missing: Vec<String> = matched
            .into_iter()
            .filter(|fqcn| !schemas.contains_key(fqcn))
            .take(4)
            .collect();

        if !missing.is_empty() {
            step_counter += 1;
            let step_id = format!("pre-{step_counter}");
            let short_names: Vec<&str> = missing
                .iter()
                .map(|m| m.rsplit('\\').next().unwrap_or(m.as_str()))
                .collect();
            let _ = tx.send(AiUiEvent::TurnToolStep {
                msg_id: input.msg_id,
                step: AiToolStep {
                    id: step_id.clone(),
                    summary: format!("Inspecting {} schema…", short_names.join(", ")),
                    done: false,
                    is_error: false,
                },
            });

            if let Ok(resp) = introspect_laravel_schema(
                &SchemaIntrospectRequest {
                    models: missing,
                    tables: Vec::new(),
                    list_tables: false,
                },
                &input.target,
                Some(&input.php_binary),
            ) {
                let mut col_summary = Vec::new();
                for m in resp.models {
                    if m.error.is_none() {
                        let short = m.fqcn.rsplit('\\').next().unwrap_or(&m.fqcn);
                        col_summary.push(format!("{short} ({} cols)", m.columns.len()));
                        schemas.insert(m.fqcn.clone(), m.clone());
                        newly_cached_models.push(m);
                    }
                }
                if !col_summary.is_empty() {
                    let _ = tx.send(AiUiEvent::TurnToolStep {
                        msg_id: input.msg_id,
                        step: AiToolStep {
                            id: step_id,
                            summary: format!("Inspected {}", col_summary.join(", ")),
                            done: true,
                            is_error: false,
                        },
                    });
                }
            }
        }
    }

    let mut tool_history_transcript = String::new();

    for round in 0..=MAX_TOOL_ROUNDS {
        if cancel.load(Ordering::SeqCst) {
            let _ = tx.send(AiUiEvent::TurnCancelled {
                msg_id: input.msg_id,
            });
            return;
        }

        let instructions = build_system_instructions(
            &input,
            &schemas,
            all_tables.as_deref(),
            can_introspect_db && round < MAX_TOOL_ROUNDS,
        );
        let turn_prompt = build_turn_prompt(&input, &tool_history_transcript);

        let mut streamed_so_far = String::new();
        let turn_res = run_cli_turn(
            input.provider,
            &input.model,
            input.effort.as_deref(),
            &instructions,
            &turn_prompt,
            cancel,
            TURN_TIMEOUT,
            |delta| {
                streamed_so_far.push_str(delta);
                if !looks_like_raw_tool_json(&streamed_so_far) {
                    let _ = tx.send(AiUiEvent::TurnStreamDelta {
                        msg_id: input.msg_id,
                        full_text: streamed_so_far.clone(),
                    });
                }
            },
        );

        let response_text = match turn_res {
            Ok(txt) => txt,
            Err(err) if err == "__CANCELLED__" || cancel.load(Ordering::SeqCst) => {
                let _ = tx.send(AiUiEvent::TurnCancelled {
                    msg_id: input.msg_id,
                });
                return;
            }
            Err(err) => {
                let _ = tx.send(AiUiEvent::TurnFailed {
                    msg_id: input.msg_id,
                    error: err,
                });
                return;
            }
        };

        // Check if the agent requested a read-only schema/model inspection tool
        if round < MAX_TOOL_ROUNDS
            && let Some(tool_call) = parse_tool_call(&response_text)
        {
            let _ = tx.send(AiUiEvent::TurnResetStream {
                msg_id: input.msg_id,
            });
            step_counter += 1;
            let step_id = format!("tool-{step_counter}");

            let tool_output = execute_readonly_tool(
                &tool_call,
                &input,
                can_introspect_db,
                &mut schemas,
                &mut newly_cached_models,
                &mut all_tables,
                &step_id,
                tx,
            );

            tool_history_transcript.push_str(&format!(
                "\n<assistant_tool_call>\n{}\n</assistant_tool_call>\n<tool_result>\n{}\n</tool_result>\n",
                response_text.trim(),
                tool_output
            ));
            continue;
        }

        // Final answer path: extract explanation + PHP code block
        let (mut explanation, mut php_code) = extract_php_and_explanation(&response_text);
        let mut lint_passed = true;

        // Validate PHP syntax with `php -l`; perform 1 automatic repair pass if invalid
        if let Some(ref code) = php_code
            && let Some(diag) = lint_php_syntax_with_bin(code, Some(&input.php_binary))
        {
            lint_passed = false;
            step_counter += 1;
            let repair_id = format!("lint-{step_counter}");
            let _ = tx.send(AiUiEvent::TurnToolStep {
                msg_id: input.msg_id,
                step: AiToolStep {
                    id: repair_id.clone(),
                    summary: format!("Auto-fixing PHP syntax on line {}…", diag.line),
                    done: false,
                    is_error: false,
                },
            });

            let repair_prompt = format!(
                "{turn_prompt}\n\nYour previous PHP snippet had a syntax error on line {}: {}\nPrevious PHP code:\n```php\n{code}\n```\nReturn the corrected response with a valid ```php``` block.",
                diag.line, diag.message
            );
            let mut repaired_stream = String::new();
            let _ = tx.send(AiUiEvent::TurnResetStream {
                msg_id: input.msg_id,
            });
            if let Ok(repaired_txt) = run_cli_turn(
                input.provider,
                &input.model,
                input.effort.as_deref(),
                &instructions,
                &repair_prompt,
                cancel,
                TURN_TIMEOUT,
                |delta| {
                    repaired_stream.push_str(delta);
                    let _ = tx.send(AiUiEvent::TurnStreamDelta {
                        msg_id: input.msg_id,
                        full_text: repaired_stream.clone(),
                    });
                },
            ) {
                let (rep_exp, rep_code) = extract_php_and_explanation(&repaired_txt);
                if let Some(rc) = rep_code {
                    let ok = lint_php_syntax_with_bin(&rc, Some(&input.php_binary)).is_none();
                    lint_passed = ok;
                    php_code = Some(rc);
                    if !rep_exp.is_empty() {
                        explanation = rep_exp;
                    }
                    let _ = tx.send(AiUiEvent::TurnToolStep {
                        msg_id: input.msg_id,
                        step: AiToolStep {
                            id: repair_id,
                            summary: if ok {
                                "Verified PHP syntax (php -l)".to_string()
                            } else {
                                "Syntax warning remains".to_string()
                            },
                            done: true,
                            is_error: !ok,
                        },
                    });
                }
            }
        }

        let _ = tx.send(AiUiEvent::TurnCompleted {
            msg_id: input.msg_id,
            explanation,
            php_code,
            lint_passed,
            project_key: input.project_path.clone(),
            cached_models: newly_cached_models,
            cached_tables: all_tables,
        });
        return;
    }

    let _ = tx.send(AiUiEvent::TurnFailed {
        msg_id: input.msg_id,
        error: "AI agent exceeded maximum schema inspection rounds without returning final code."
            .to_string(),
    });
}

#[allow(clippy::too_many_arguments)]
fn execute_readonly_tool(
    call: &ParsedToolCall,
    input: &AiHarnessJobInput,
    can_introspect_db: bool,
    schemas: &mut HashMap<String, ModelSchemaInfo>,
    newly_cached_models: &mut Vec<ModelSchemaInfo>,
    all_tables: &mut Option<Vec<String>>,
    step_id: &str,
    tx: &Sender<AiUiEvent>,
) -> String {
    match call {
        ParsedToolCall::InspectModels(models) => {
            if !can_introspect_db {
                let _ = tx.send(AiUiEvent::TurnToolStep {
                    msg_id: input.msg_id,
                    step: AiToolStep {
                        id: step_id.to_string(),
                        summary: "SSH schema inspection disabled in Settings".to_string(),
                        done: true,
                        is_error: true,
                    },
                });
                return json!({
                    "error": "Live SSH schema inspection is disabled in Laralah Settings -> AI Assistant."
                })
                .to_string();
            }

            let capped: Vec<String> = models.iter().take(8).cloned().collect();
            let label_list: Vec<&str> = capped
                .iter()
                .map(|m| m.rsplit('\\').next().unwrap_or(m.as_str()))
                .collect();
            let _ = tx.send(AiUiEvent::TurnToolStep {
                msg_id: input.msg_id,
                step: AiToolStep {
                    id: step_id.to_string(),
                    summary: format!("Inspecting {}…", label_list.join(", ")),
                    done: false,
                    is_error: false,
                },
            });

            match introspect_laravel_schema(
                &SchemaIntrospectRequest {
                    models: capped,
                    tables: Vec::new(),
                    list_tables: false,
                },
                &input.target,
                Some(&input.php_binary),
            ) {
                Ok(resp) => {
                    let mut summaries = Vec::new();
                    for m in &resp.models {
                        let short = m.fqcn.rsplit('\\').next().unwrap_or(&m.fqcn);
                        if m.error.is_none() {
                            summaries.push(format!("{short} ({} cols)", m.columns.len()));
                            schemas.insert(m.fqcn.clone(), m.clone());
                            newly_cached_models.push(m.clone());
                        } else {
                            summaries.push(format!("{short} (not found)"));
                        }
                    }
                    let _ = tx.send(AiUiEvent::TurnToolStep {
                        msg_id: input.msg_id,
                        step: AiToolStep {
                            id: step_id.to_string(),
                            summary: format!("Inspected {}", summaries.join(", ")),
                            done: true,
                            is_error: false,
                        },
                    });
                    serde_json::to_string(&resp.models).unwrap_or_else(|_| "[]".to_string())
                }
                Err(err) => {
                    let _ = tx.send(AiUiEvent::TurnToolStep {
                        msg_id: input.msg_id,
                        step: AiToolStep {
                            id: step_id.to_string(),
                            summary: "Schema inspection failed".to_string(),
                            done: true,
                            is_error: true,
                        },
                    });
                    json!({ "error": format!("{err:#}") }).to_string()
                }
            }
        }
        ParsedToolCall::ListTables => {
            if !can_introspect_db {
                return json!({ "error": "SSH schema inspection disabled." }).to_string();
            }
            let _ = tx.send(AiUiEvent::TurnToolStep {
                msg_id: input.msg_id,
                step: AiToolStep {
                    id: step_id.to_string(),
                    summary: "Listing database tables…".to_string(),
                    done: false,
                    is_error: false,
                },
            });
            match introspect_laravel_schema(
                &SchemaIntrospectRequest {
                    models: Vec::new(),
                    tables: Vec::new(),
                    list_tables: true,
                },
                &input.target,
                Some(&input.php_binary),
            ) {
                Ok(resp) => {
                    let count = resp.all_tables.len();
                    *all_tables = Some(resp.all_tables.clone());
                    let _ = tx.send(AiUiEvent::TurnToolStep {
                        msg_id: input.msg_id,
                        step: AiToolStep {
                            id: step_id.to_string(),
                            summary: format!("Listed {count} database tables"),
                            done: true,
                            is_error: false,
                        },
                    });
                    json!({ "tables": resp.all_tables }).to_string()
                }
                Err(err) => json!({ "error": format!("{err:#}") }).to_string(),
            }
        }
        ParsedToolCall::InspectTables(tables) => {
            if !can_introspect_db {
                return json!({ "error": "SSH schema inspection disabled." }).to_string();
            }
            let capped: Vec<String> = tables.iter().take(8).cloned().collect();
            let _ = tx.send(AiUiEvent::TurnToolStep {
                msg_id: input.msg_id,
                step: AiToolStep {
                    id: step_id.to_string(),
                    summary: format!("Inspecting table(s) {}…", capped.join(", ")),
                    done: false,
                    is_error: false,
                },
            });
            match introspect_laravel_schema(
                &SchemaIntrospectRequest {
                    models: Vec::new(),
                    tables: capped.clone(),
                    list_tables: false,
                },
                &input.target,
                Some(&input.php_binary),
            ) {
                Ok(resp) => {
                    let _ = tx.send(AiUiEvent::TurnToolStep {
                        msg_id: input.msg_id,
                        step: AiToolStep {
                            id: step_id.to_string(),
                            summary: format!("Inspected table(s) {}", capped.join(", ")),
                            done: true,
                            is_error: false,
                        },
                    });
                    serde_json::to_string(&resp.tables).unwrap_or_else(|_| "[]".to_string())
                }
                Err(err) => json!({ "error": format!("{err:#}") }).to_string(),
            }
        }
        ParsedToolCall::ReadModelSource(model_name) => {
            let short = model_name
                .rsplit('\\')
                .next()
                .unwrap_or(model_name.as_str());
            let candidates = [
                Path::new(&input.project_path)
                    .join("app")
                    .join("Models")
                    .join(format!("{short}.php")),
                Path::new(&input.project_path)
                    .join("app")
                    .join(format!("{short}.php")),
            ];
            for path in candidates {
                if let Ok(src) = std::fs::read_to_string(&path) {
                    let truncated: String = src.chars().take(6000).collect();
                    let _ = tx.send(AiUiEvent::TurnToolStep {
                        msg_id: input.msg_id,
                        step: AiToolStep {
                            id: step_id.to_string(),
                            summary: format!("Read {short}.php source"),
                            done: true,
                            is_error: false,
                        },
                    });
                    return json!({ "model": model_name, "source": truncated }).to_string();
                }
            }
            let _ = tx.send(AiUiEvent::TurnToolStep {
                msg_id: input.msg_id,
                step: AiToolStep {
                    id: step_id.to_string(),
                    summary: format!("{short}.php not found locally"),
                    done: true,
                    is_error: true,
                },
            });
            json!({ "error": format!("Model source file for '{model_name}' not found locally.") })
                .to_string()
        }
    }
}

fn match_prompt_models(prompt: &str, indexed_models: &[String]) -> Vec<String> {
    let lower_prompt = prompt.to_lowercase();
    let words: HashSet<String> = lower_prompt
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(str::to_string)
        .collect();

    let mut matched = Vec::new();
    for fqcn in indexed_models {
        let short = fqcn.rsplit('\\').next().unwrap_or(fqcn);
        let short_lower = short.to_lowercase();
        let plural_s = format!("{short_lower}s");
        let plural_ies = short_lower
            .strip_suffix('y')
            .map(|stem| format!("{stem}ies"))
            .unwrap_or_default();
        let at_mention = format!("@{short_lower}");

        if lower_prompt.contains(&at_mention)
            || words.contains(&short_lower)
            || words.contains(&plural_s)
            || (!plural_ies.is_empty() && words.contains(&plural_ies))
        {
            matched.push(fqcn.clone());
        }
    }
    matched
}

fn format_schema_for_prompt(schema: &ModelSchemaInfo) -> String {
    let cols: Vec<String> = schema
        .columns
        .iter()
        .map(|c| {
            let null_hint = if c.nullable { "?" } else { "" };
            format!("{}:{}{}", c.name, c.col_type, null_hint)
        })
        .collect();
    let rels: Vec<String> = schema
        .relations
        .iter()
        .map(|r| match &r.related {
            Some(rel) => format!("{}(): {}<{}>", r.name, r.kind, rel),
            None => format!("{}(): {}", r.name, r.kind),
        })
        .collect();
    let casts: Vec<String> = schema
        .casts
        .iter()
        .map(|(k, v)| format!("{k}=>{v}"))
        .collect();

    let mut line = format!(
        "- {} (table: `{}`, pk: `{}`{}{})\n  columns: [{}]",
        schema.fqcn,
        schema.table,
        schema.primary_key,
        if schema.soft_deletes {
            ", SoftDeletes"
        } else {
            ""
        },
        if !schema.timestamps {
            ", timestamps=false"
        } else {
            ""
        },
        cols.join(", ")
    );
    if !casts.is_empty() {
        line.push_str(&format!("\n  casts: [{}]", casts.join(", ")));
    }
    if !schema.scopes.is_empty() {
        line.push_str(&format!("\n  scopes: [{}]", schema.scopes.join(", ")));
    }
    if !rels.is_empty() {
        line.push_str(&format!("\n  relations: [{}]", rels.join(", ")));
    }
    line
}

fn build_system_instructions(
    input: &AiHarnessJobInput,
    schemas: &HashMap<String, ModelSchemaInfo>,
    all_tables: Option<&[String]>,
    tools_enabled: bool,
) -> String {
    let target_desc = match &input.target {
        ExecutionTarget::Local { .. } => "Local Laravel project".to_string(),
        ExecutionTarget::Docker { container_name, .. } => {
            format!("Docker container ({container_name})")
        }
        ExecutionTarget::Ssh {
            name,
            docker_container: Some(container),
            ..
        } if !container.is_empty() => {
            format!("Remote SSH server ({name}) -> Docker container ({container})")
        }
        ExecutionTarget::Ssh { name, .. } => format!("Remote SSH server ({name})"),
    };

    let models_list = if input.indexed_models.is_empty() {
        "No Eloquent models indexed yet (use list_tables or inspect_models if needed).".to_string()
    } else {
        input
            .indexed_models
            .iter()
            .take(160)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut verified_block = String::new();
    if !schemas.is_empty() {
        let mut keys: Vec<&String> = schemas.keys().collect();
        keys.sort();
        for k in keys {
            if let Some(s) = schemas.get(k) {
                verified_block.push_str(&format_schema_for_prompt(s));
                verified_block.push('\n');
            }
        }
    }
    if let Some(tables) = all_tables
        && !tables.is_empty()
    {
        verified_block.push_str(&format!("Database tables: {}\n", tables.join(", ")));
    }

    let tool_section = if tools_enabled {
        r#"## Schema & Model Introspection Tools (Read-Only)
Never guess database column names, foreign keys, or enum values.
If you need columns/relations for a model or table NOT already listed under `<verified_schemas>`, respond with ONLY a single JSON tool request object (no prose, no PHP code yet) using one of these shapes:
- `{"tool": "inspect_models", "models": ["App\\Models\\User", "App\\Models\\Order"]}`
- `{"tool": "list_tables"}`
- `{"tool": "inspect_tables", "tables": ["role_user"]}`
- `{"tool": "read_model_source", "model": "App\\Models\\User"}`

Laralah will execute that read-only inspection and reply with `<tool_result>` so you can write exact Eloquent code."#
    } else {
        "Write the best Eloquent / Laravel PHP code using the verified schemas and models provided."
    };

    format!(
        r#"You are Laralah's built-in Laravel & Eloquent coding assistant.
Your job is to answer the user's request with accurate, idiomatic PHP / Laravel Eloquent code ready to run inside Laralah's interactive REPL scratchpad.

## Active Environment
- Framework: {laravel}
- PHP: {php}
- Target: {target_desc}

## Available Eloquent Models
{models_list}

<verified_schemas>
{verified}
</verified_schemas>

{tool_section}

## Final Answer Rules (when you have enough schema info)
1. Keep any explanation to 1–2 concise sentences maximum.
2. Put the executable PHP snippet inside a single ```php ... ``` fenced block.
3. Omit `<?php` and `declare(...)` tags — Laralah's scratchpad evaluates raw PHP statements directly.
4. Import referenced classes at the top with `use ...;` (e.g. `use App\Models\User;`).
5. Prefer expressive Eloquent & Carbon helpers (`today()->subDay()`, `whereDate(...)`, `withCount(...)`, scopes, relations).
6. End the snippet with the final expression to inspect so Laralah's Detail Dive / Table view renders the result cleanly."#,
        laravel = input.laravel_version,
        php = input.php_version,
        verified = if verified_block.trim().is_empty() {
            "(None loaded yet — call inspect_models or list_tables if you need column names.)"
        } else {
            verified_block.trim()
        },
    )
}

fn build_turn_prompt(input: &AiHarnessJobInput, tool_transcript: &str) -> String {
    let mut prompt = String::new();

    if !input.prior_turns.is_empty() {
        prompt.push_str("<conversation_history>\n");
        for (u, a) in input.prior_turns.iter().rev().take(4).rev() {
            prompt.push_str(&format!("User: {u}\nAssistant:\n{a}\n\n"));
        }
        prompt.push_str("</conversation_history>\n\n");
    }

    if let Some(ref code) = input.editor_code
        && !code.trim().is_empty()
    {
        let capped: String = code.chars().take(4000).collect();
        prompt.push_str(&format!(
            "<current_editor_code>\n{capped}\n</current_editor_code>\n\n"
        ));
    }

    if let Some(ref exc) = input.last_exception
        && !exc.trim().is_empty()
    {
        prompt.push_str(&format!(
            "<last_execution_error>\n{exc}\n</last_execution_error>\n\n"
        ));
    }

    prompt.push_str(&format!("User request: {}\n", input.user_prompt.trim()));

    if !tool_transcript.is_empty() {
        prompt.push_str("\n--- Previous Tool Inspection Steps ---");
        prompt.push_str(tool_transcript);
        prompt.push_str("\nContinue: either request one more tool if still needed, or provide the final concise answer + ```php``` block.");
    }

    prompt
}

fn looks_like_raw_tool_json(text: &str) -> bool {
    let trimmed = text.trim_start();
    if let Some(after_fence) = trimmed.strip_prefix("```json") {
        let inner = after_fence.trim_start();
        return inner.is_empty() || inner.starts_with('{');
    }
    trimmed.starts_with('{') && (trimmed.len() < 24 || trimmed.contains("\"tool\""))
}

fn parse_tool_call(response: &str) -> Option<ParsedToolCall> {
    let trimmed = response.trim();
    // If response already contains a ```php block, treat it as a final answer, not a tool call.
    if trimmed.contains("```php") || trimmed.contains("```PHP") {
        return None;
    }

    let candidate = if trimmed.starts_with("```") {
        let without_open = trimmed.split_once('\n').map(|(_, r)| r).unwrap_or("");
        without_open
            .rsplit_once("```")
            .map(|(h, _)| h.trim())
            .unwrap_or(without_open.trim())
    } else if let (Some(s), Some(e)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if s == 0 && e + 1 == trimmed.len() {
            &trimmed[s..=e]
        } else {
            return None;
        }
    } else {
        return None;
    };

    let v: Value = serde_json::from_str(candidate).ok()?;
    let tool_name = v["tool"].as_str()?;
    let args = v.get("args").unwrap_or(&v);

    match tool_name {
        "inspect_models" | "inspect_model" => {
            let mut list: Vec<String> = args["models"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if list.is_empty()
                && let Some(single) = args["model"].as_str()
                && !single.trim().is_empty()
            {
                list.push(single.trim().to_string());
            }
            if list.is_empty() {
                None
            } else {
                Some(ParsedToolCall::InspectModels(list))
            }
        }
        "list_tables" => Some(ParsedToolCall::ListTables),
        "inspect_tables" | "inspect_table" => {
            let mut list: Vec<String> = args["tables"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if list.is_empty()
                && let Some(single) = args["table"].as_str()
                && !single.trim().is_empty()
            {
                list.push(single.trim().to_string());
            }
            if list.is_empty() {
                None
            } else {
                Some(ParsedToolCall::InspectTables(list))
            }
        }
        "read_model_source" => args["model"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| ParsedToolCall::ReadModelSource(s.to_string())),
        _ => None,
    }
}

pub fn clean_php_snippet(raw: &str) -> String {
    let mut text = raw.replace("\r\n", "\n").trim().to_string();
    if let Some(rest) = text.strip_prefix("<?php") {
        text = rest.trim_start().to_string();
    } else if let Some(rest) = text.strip_prefix("<?") {
        text = rest.trim_start().to_string();
    }
    if let Some(rest) = text.strip_prefix("declare(strict_types=1);") {
        text = rest.trim_start().to_string();
    }
    text.trim().to_string()
}

pub fn extract_php_and_explanation(raw: &str) -> (String, Option<String>) {
    let trimmed = raw.trim();
    if let Some(caps) = CODE_BLOCK_RE.captures(trimmed)
        && let Some(code_match) = caps.get(1)
    {
        let cleaned_code = clean_php_snippet(code_match.as_str());
        let full_match = caps.get(0).unwrap();
        let before = trimmed[..full_match.start()].trim();
        let after = trimmed[full_match.end()..].trim();
        let explanation = match (before.is_empty(), after.is_empty()) {
            (true, true) => String::new(),
            (false, true) => before.to_string(),
            (true, false) => after.to_string(),
            (false, false) => format!("{before}\n{after}"),
        };
        if !cleaned_code.is_empty() {
            return (explanation, Some(cleaned_code));
        }
    }

    // Fallback: if the model returned pure PHP code without markdown fences
    if (trimmed.starts_with("use ")
        || trimmed.starts_with('$')
        || trimmed.contains("::")
        || trimmed.starts_with("<?php"))
        && !trimmed.contains('\n')
        || trimmed.starts_with("<?php")
    {
        let cleaned = clean_php_snippet(trimmed);
        if !cleaned.is_empty() && lint_php_syntax_with_bin(&cleaned, None).is_none() {
            return (String::new(), Some(cleaned));
        }
    }

    (trimmed.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_models_mentioned_in_natural_language_prompt() {
        let indexed = vec![
            "App\\Models\\User".to_string(),
            "App\\Models\\Article".to_string(),
            "App\\Models\\Category".to_string(),
        ];
        let hit = match_prompt_models("How many user register at yesterday", &indexed);
        assert_eq!(hit, vec!["App\\Models\\User".to_string()]);

        let plural_hit = match_prompt_models("Count categories with articles", &indexed);
        assert!(plural_hit.contains(&"App\\Models\\Category".to_string()));
        assert!(plural_hit.contains(&"App\\Models\\Article".to_string()));
    }

    #[test]
    fn parses_tool_calls_and_extracts_clean_php_code() {
        let call = parse_tool_call(r#"{"tool":"inspect_models","models":["App\\Models\\User"]}"#);
        assert_eq!(
            call,
            Some(ParsedToolCall::InspectModels(vec![
                "App\\Models\\User".to_string()
            ]))
        );

        let fenced_call = parse_tool_call(
            "```json\n{\"tool\": \"inspect_tables\", \"args\": {\"tables\": [\"role_user\"]}}\n```",
        );
        assert_eq!(
            fenced_call,
            Some(ParsedToolCall::InspectTables(vec![
                "role_user".to_string()
            ]))
        );

        let (exp, code) = extract_php_and_explanation(
            "Counting users registered yesterday:\n```php\n<?php\nuse App\\Models\\User;\n\nUser::whereDate('created_at', today()->subDay())->count();\n```",
        );
        assert_eq!(exp, "Counting users registered yesterday:");
        assert_eq!(
            code.as_deref(),
            Some("use App\\Models\\User;\n\nUser::whereDate('created_at', today()->subDay())->count();")
        );
    }
}
