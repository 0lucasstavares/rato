use anyhow::Result;

use crate::intent::Lang;

#[derive(Debug, Clone, PartialEq)]
pub enum BackendHealth {
    Ok,
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WakeHit {
    pub word: String,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub lang: Lang,
}

pub trait AudioSource {
    fn health(&self) -> BackendHealth;
    fn next_frame(&mut self) -> Result<Option<Vec<f32>>>;
}

pub trait WakeDetector {
    fn health(&self) -> BackendHealth;
    fn detect(&mut self, frame: &[f32]) -> Result<Option<WakeHit>>;
}

pub trait Vad {
    fn health(&self) -> BackendHealth;
    fn is_speech(&mut self, frame: &[f32]) -> Result<bool>;
}

pub trait SttEngine {
    fn health(&self) -> BackendHealth;
    fn transcribe(&mut self, pcm: &[f32]) -> Result<Transcript>;
}

pub trait TtsEngine {
    fn health(&self) -> BackendHealth;
    fn speak(&mut self, text: &str, lang: Lang) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct FakeAudioSource {
    frames: std::collections::VecDeque<Vec<f32>>,
}

impl FakeAudioSource {
    pub fn new(frames: Vec<Vec<f32>>) -> Self {
        Self {
            frames: frames.into(),
        }
    }
}

impl AudioSource for FakeAudioSource {
    fn health(&self) -> BackendHealth {
        BackendHealth::Ok
    }

    fn next_frame(&mut self) -> Result<Option<Vec<f32>>> {
        Ok(self.frames.pop_front())
    }
}

#[derive(Debug, Clone)]
pub struct FakeWakeDetector {
    fire_on: usize,
    seen: usize,
    word: String,
}

impl FakeWakeDetector {
    pub fn fire_on(fire_on: usize, word: impl Into<String>) -> Self {
        Self {
            fire_on,
            seen: 0,
            word: word.into(),
        }
    }
}

impl WakeDetector for FakeWakeDetector {
    fn health(&self) -> BackendHealth {
        BackendHealth::Ok
    }

    fn detect(&mut self, _frame: &[f32]) -> Result<Option<WakeHit>> {
        let hit = self.seen == self.fire_on;
        self.seen += 1;
        Ok(hit.then(|| WakeHit {
            word: self.word.clone(),
            score: 1.0,
        }))
    }
}

#[derive(Debug, Clone)]
pub struct FakeVad {
    speech: bool,
}

impl FakeVad {
    pub fn new(speech: bool) -> Self {
        Self { speech }
    }
}

impl Vad for FakeVad {
    fn health(&self) -> BackendHealth {
        BackendHealth::Ok
    }

    fn is_speech(&mut self, _frame: &[f32]) -> Result<bool> {
        Ok(self.speech)
    }
}

#[derive(Debug, Clone)]
pub struct FakeStt {
    transcript: Transcript,
}

impl FakeStt {
    pub fn new(text: impl Into<String>, lang: Lang) -> Self {
        Self {
            transcript: Transcript {
                text: text.into(),
                lang,
            },
        }
    }
}

impl SttEngine for FakeStt {
    fn health(&self) -> BackendHealth {
        BackendHealth::Ok
    }

    fn transcribe(&mut self, _pcm: &[f32]) -> Result<Transcript> {
        Ok(self.transcript.clone())
    }
}

#[derive(Debug, Default)]
pub struct FakeTts {
    pub spoken: Vec<Transcript>,
}

impl TtsEngine for FakeTts {
    fn health(&self) -> BackendHealth {
        BackendHealth::Ok
    }

    fn speak(&mut self, text: &str, lang: Lang) -> Result<()> {
        self.spoken.push(Transcript {
            text: text.to_string(),
            lang,
        });
        Ok(())
    }
}
