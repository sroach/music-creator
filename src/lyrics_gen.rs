use foundation_models::{Availability, FMError, LanguageModelSession, StreamEvent, SystemLanguageModel, Unavailability};

/// Wraps a Foundation Models language model session for lyrics generation.
pub struct LyricsGenerator {
    session: LanguageModelSession,
}

impl LyricsGenerator {
    /// Creates a new lyrics generator with system instructions for structured
    /// song lyrics generation.
    ///
    /// The `topic` describes what the song should be about, and `style` provides
    /// additional context for tone and structure.
    pub fn new(topic: &str, style: &str) -> Result<Self, FMError> {
        let system_prompt = format!(
            "You are a skilled songwriter. Generate complete song lyrics about: {topic}.\n\
             Style context: {style}\n\
             \n\
             First, output a metadata block at the very beginning, enclosed in [[METADATA]] tags:\n\
             [[METADATA]]\n\
             DURATION: <seconds, e.g. 180>\n\
             BPM: <beats per minute, e.g. 120>\n\
             STYLE: <a concise style description, e.g. indie pop, soft male vocal, warm acoustic guitar>\n\
             [[END_METADATA]]\n\
             \n\
             Then, structure the lyrics with proper song sections using these tags:\n\
             - [verse] for verses\n\
             - [chorus] for the chorus (should be catchy and repeatable)\n\
             - [bridge] for a bridge section (optional but recommended)\n\
             - [outro] for the ending\n\
             \n\
             Include at least 2 verses, 2 choruses, and optionally a bridge and outro.\n\
             Make the lyrics creative, emotional, and suitable for a song.",
            topic = topic,
            style = style
        );

        let session = LanguageModelSession::with_instructions(&system_prompt)?;
        Ok(Self { session })
    }

    /// Streams the generated lyrics, calling the callback for each stream event.
    pub fn stream<F>(&self, prompt: &str, callback: F) -> Result<(), FMError>
    where
        F: FnMut(StreamEvent<'_>) + Send + 'static,
    {
        self.session.stream(prompt, callback)
    }

    /// Gets a complete response (blocking, non-streaming).
    pub fn respond(&self, prompt: &str) -> Result<String, FMError> {
        self.session.respond(prompt)
    }
}

/// Checks if the Foundation Models framework is available on this system.
pub fn is_available() -> bool {
    SystemLanguageModel::is_available()
}

/// Returns a human-readable description of why the model might be unavailable.
pub fn availability_reason() -> String {
    let avail = SystemLanguageModel::availability();
    match avail {
        Availability::Available => {
            "Foundation Models is available.".to_string()
        }
        Availability::Unavailable(reason) => {
            match reason {
                Unavailability::DeviceNotEligible => {
                    "Device does not support Apple Intelligence.".to_string()
                }
                Unavailability::AppleIntelligenceNotEnabled => {
                    "Apple Intelligence is disabled in System Settings. Please enable it.".to_string()
                }
                Unavailability::ModelNotReady => {
                    "Model assets are still downloading.".to_string()
                }
                Unavailability::OsTooOld => {
                    "macOS 26 Tahoe or later is required.".to_string()
                }
                Unavailability::Unknown => {
                    "Foundation Models is not available for an unknown reason.".to_string()
                }
                _ => "Foundation Models is not available.".to_string(),
            }
        }
        _ => "Foundation Models is not available.".to_string(),
    }
}
