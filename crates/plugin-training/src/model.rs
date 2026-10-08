//! Training data shapes: exercises, logged sessions and their validation.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExerciseKind {
    Strength,
    Cardio,
    /// Stretching, yoga, mobility work.
    Mobility,
    Sport,
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intensity {
    Low,
    #[default]
    Moderate,
    High,
}

impl Intensity {
    pub fn zh_label(self) -> &'static str {
        match self {
            Intensity::Low => "低强度",
            Intensity::Moderate => "中等强度",
            Intensity::High => "高强度",
        }
    }
}

/// One exercise, logged or planned. Strength work usually has `sets`/`reps`/`weight_kg`,
/// cardio `duration_min`/`distance_km`; every number is optional, and the app fills in a
/// duration when none is given (`energy::exercise_minutes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exercise {
    pub name: String,
    pub kind: ExerciseKind,
    #[serde(default)]
    pub intensity: Intensity,
    #[serde(default)]
    pub sets: Option<u32>,
    #[serde(default)]
    pub reps: Option<u32>,
    #[serde(default)]
    pub weight_kg: Option<f64>,
    #[serde(default)]
    pub duration_min: Option<f64>,
    #[serde(default)]
    pub distance_km: Option<f64>,
    #[serde(default)]
    pub note: Option<String>,
}

pub const MAX_NAME_CHARS: usize = 60;

/// Why an exercise fails [`Exercise::check`]. The serde name is what the UI words
/// (`planNotes.training_invalid_exercise`); `Display` is English for the LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExerciseProblem {
    NameEmpty,
    NameTooLong,
    Sets,
    Reps,
    Weight,
    Duration,
    Distance,
}

impl std::fmt::Display for ExerciseProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameEmpty => f.write_str("the name must not be empty"),
            Self::NameTooLong => write!(f, "the name must be at most {MAX_NAME_CHARS} characters"),
            Self::Sets => f.write_str("sets must be 1–20"),
            Self::Reps => f.write_str("reps must be 1–200"),
            Self::Weight => f.write_str("weight must be 0–500 kg"),
            Self::Duration => f.write_str("duration must be 0–300 minutes"),
            Self::Distance => f.write_str("distance must be 0–300 km"),
        }
    }
}

impl Exercise {
    /// Range checks on everything the user or the planner can get wrong.
    pub fn check(&self) -> Result<(), ExerciseProblem> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ExerciseProblem::NameEmpty);
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(ExerciseProblem::NameTooLong);
        }
        let in_range = |v: Option<f64>, lo: f64, hi: f64| v.is_none_or(|v| v > lo && v <= hi);
        if !self.sets.is_none_or(|s| (1..=20).contains(&s)) {
            return Err(ExerciseProblem::Sets);
        }
        if !self.reps.is_none_or(|r| (1..=200).contains(&r)) {
            return Err(ExerciseProblem::Reps);
        }
        if !self.weight_kg.is_none_or(|w| (0.0..=500.0).contains(&w)) {
            return Err(ExerciseProblem::Weight);
        }
        if !in_range(self.duration_min, 0.0, 300.0) {
            return Err(ExerciseProblem::Duration);
        }
        if !in_range(self.distance_km, 0.0, 300.0) {
            return Err(ExerciseProblem::Distance);
        }
        Ok(())
    }

    /// "深蹲 4×8 60kg" / "跑步 30分钟 5km", for context snippets and briefs.
    pub fn label(&self) -> String {
        let mut parts = vec![self.name.trim().to_string()];
        match (self.sets, self.reps) {
            (Some(s), Some(r)) => parts.push(format!("{s}×{r}")),
            (Some(s), None) => parts.push(format!("{s}组")),
            (None, Some(r)) => parts.push(format!("{r}次")),
            (None, None) => {}
        }
        if let Some(w) = self.weight_kg.filter(|w| *w > 0.0) {
            parts.push(format!("{w}kg"));
        }
        if let Some(d) = self.duration_min {
            parts.push(format!("{d:.0}分钟"));
        }
        if let Some(km) = self.distance_km {
            parts.push(format!("{km}km"));
        }
        parts.join(" ")
    }
}

/// The payload of a `training.session_logged` event. `duration_min` and `exercise_kcal` are
/// computed by the app at logging time (`energy`), never taken from the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionLogged {
    pub title: String,
    /// What the user said, for later reference.
    #[serde(default)]
    pub raw_text: String,
    pub exercises: Vec<Exercise>,
    pub duration_min: f64,
    /// Net energy above rest; `None` when no weight was recorded to compute it from.
    pub exercise_kcal: Option<f64>,
    /// The hardest exercise's intensity.
    pub intensity: Intensity,
    /// Rate of perceived exertion, 1–10, if the user gave one.
    #[serde(default)]
    pub rpe: Option<u8>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn exercise(name: &str, kind: ExerciseKind) -> Exercise {
        Exercise {
            name: name.into(),
            kind,
            intensity: Intensity::Moderate,
            sets: None,
            reps: None,
            weight_kg: None,
            duration_min: None,
            distance_km: None,
            note: None,
        }
    }

    #[test]
    fn check_rejects_out_of_range_values() {
        let ok = Exercise {
            sets: Some(4),
            reps: Some(8),
            weight_kg: Some(60.0),
            ..exercise("深蹲", ExerciseKind::Strength)
        };
        assert_eq!(ok.check(), Ok(()));
        assert!(exercise(" ", ExerciseKind::Strength).check().is_err());
        assert!(Exercise {
            sets: Some(0),
            ..ok.clone()
        }
        .check()
        .is_err());
        assert!(Exercise {
            duration_min: Some(0.0),
            ..ok.clone()
        }
        .check()
        .is_err());
        assert!(Exercise {
            distance_km: Some(500.0),
            ..ok
        }
        .check()
        .is_err());
    }

    #[test]
    fn label_lists_the_given_numbers() {
        let squat = Exercise {
            sets: Some(4),
            reps: Some(8),
            weight_kg: Some(60.0),
            ..exercise("深蹲", ExerciseKind::Strength)
        };
        assert_eq!(squat.label(), "深蹲 4×8 60kg");
        let run = Exercise {
            duration_min: Some(30.0),
            distance_km: Some(5.0),
            ..exercise("跑步", ExerciseKind::Cardio)
        };
        assert_eq!(run.label(), "跑步 30分钟 5km");
    }

    #[test]
    fn missing_intensity_and_numbers_deserialize_as_defaults() {
        let e: Exercise =
            serde_json::from_value(serde_json::json!({"name": "跑步", "kind": "cardio"})).unwrap();
        assert_eq!(e, exercise("跑步", ExerciseKind::Cardio));
    }
}
