use serde::{Deserialize, Serialize};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use crate::ui;
use tracing::{debug, error, instrument, warn};

const DEFAULT_URL: &str = "http://localhost:11434/api/chat";
const DEFAULT_MODEL: &str = "gemma4:e2b";
const SYSTEM_PROMPT: &str = "Eres un asistente de voz que ayuda a practicar croata. \
El usuario te habla en español. Para cada mensaje devuelve un JSON con dos campos: \
\"translation\": la traducción fiel al croata (hrvatski) de lo que dijo el usuario; \
\"response\": tu respuesta al usuario, también en croata, breve y natural (una o dos frases). \
Ambos textos se leerán en voz alta: no uses markdown, listas, emojis ni símbolos.";

#[derive(Debug)]
pub enum LlmError {
    /// Ollama answered, but with an error status (e.g. 404 for an unknown model)
    Status { status: u16, body: String },
    /// Couldn't reach Ollama at all (not running, wrong URL, timeout…)
    Transport {
        url: String,
        source: Box<ureq::Transport>,
    },
    /// The HTTP response body couldn't be read/decoded
    Body(std::io::Error),
    /// The model's content didn't match the {translation, response} schema
    Format {
        raw: String,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status { status, body } => write!(f, "ollama returned {status}: {body}"),
            Self::Transport { url, source } => write!(f, "could not reach ollama at {url}: {source}"),
            Self::Body(e) => write!(f, "could not read the ollama response: {e}"),
            Self::Format { raw, source } => write!(f, "unexpected llm output ({source}): {raw}"),
        }
    }
}

impl std::error::Error for LlmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Status { .. } => None,
            Self::Transport { source, .. } => Some(source.as_ref()),
            Self::Body(e) => Some(e),
            Self::Format { source, .. } => Some(source),
        }
    }
}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        Self::Body(e)
    }
}

impl LlmError {
    fn from_ureq(err: ureq::Error, url: &str) -> Self {
        match err {
            ureq::Error::Status(status, resp) => Self::Status {
                status,
                body: resp.into_string().unwrap_or_default(),
            },
            ureq::Error::Transport(t) => Self::Transport {
                url: url.to_string(),
                source: Box::new(t),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
    /// JSON schema: Ollama constrains the output to match it
    format: serde_json::Value,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Message,
}

/// What the model returns for each thing the user says.
#[derive(Debug, Deserialize)]
pub struct Answer {
    /// The user's words translated to Croatian
    translation: String,
    /// The model's reply, in Croatian
    response: String,
}

fn answer_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "translation": { "type": "string" },
            "response": { "type": "string" }
        },
        "required": ["translation", "response"]
    })
}

/// Talks to a local Ollama server, keeping the conversation history.
pub struct LlmClient {
    agent: ureq::Agent,
    url: String,
    model: String,
    history: Vec<Message>,
}

impl LlmClient {
    pub fn new() -> Self {
        let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        debug!(%url, %model, "llm client configured");

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            // First request may have to load the model into memory
            .timeout_read(Duration::from_secs(300))
            .build();

        Self {
            agent,
            url,
            model,
            history: vec![Message {
                role: "system".into(),
                content: SYSTEM_PROMPT.into(),
            }],
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    #[instrument(skip_all)]
    pub fn ask(&mut self, prompt: &str) -> Result<Answer, LlmError> {
        self.history.push(Message {
            role: "user".into(),
            content: prompt.into(),
        });

        let start = Instant::now();
        let result = self
            .agent
            .post(&self.url)
            .send_json(ChatRequest {
                model: &self.model,
                messages: &self.history,
                stream: false,
                format: answer_schema(),
            })
            .map_err(|e| LlmError::from_ureq(e, &self.url))
            .and_then(|resp| Ok(resp.into_json::<ChatResponse>()?))
            .and_then(|resp| {
                let raw = resp.message.content;
                match serde_json::from_str::<Answer>(&raw) {
                    Ok(answer) => Ok((raw, answer)),
                    Err(source) => Err(LlmError::Format { raw, source }),
                }
            });

        match result {
            Ok((raw, answer)) => {
                debug!(
                    elapsed = ?start.elapsed(),
                    translation = %answer.translation,
                    response = %answer.response,
                    "llm replied"
                );
                // Keep the raw JSON in history so the model keeps answering in the same shape
                self.history.push(Message {
                    role: "assistant".into(),
                    content: raw,
                });
                Ok(answer)
            }
            Err(e) => {
                // Drop the unanswered turn so history stays user/assistant alternating
                self.history.pop();
                Err(e)
            }
        }
    }

    /// Receives transcripts and forwards the translation, then the response, to the
    /// talker. Returns when the input channel closes.
    pub fn run(mut self, transcripts: Receiver<String>, replies: Sender<String>) {
        for text in transcripts {
            debug!(%text, "llm received transcript");
            let start = Instant::now();
            let answer = match self.ask(&text) {
                Ok(answer) => answer,
                Err(e) => {
                    error!(error = %e, "llm request failed");
                    ui::failure(&format!("llm request failed: {e}"));
                    continue;
                }
            };
            ui::translation(answer.translation.trim());
            ui::reply(answer.response.trim(), start.elapsed());
            for part in [answer.translation, answer.response] {
                let part = part.trim().to_string();
                if part.is_empty() {
                    warn!("llm returned an empty field, skipping");
                    continue;
                }
                if replies.send(part).is_err() {
                    warn!("talker channel closed, stopping llm client");
                    return;
                }
            }
        }
        debug!("llm client stopped");
    }
}
