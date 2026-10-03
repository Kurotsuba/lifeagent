//! Duration and energy of exercises — pure functions. Energy is net of rest:
//! `(MET − 1) × kg × hours`, with MET values per kind and intensity from the Compendium of
//! Physical Activities' typical ranges.

use crate::model::{Exercise, ExerciseKind, Intensity};

/// Minutes per set of strength work, rest between sets included.
pub const MINUTES_PER_SET: f64 = 2.5;
/// Assumed duration of an exercise with neither a duration nor sets.
pub const DEFAULT_EXERCISE_MINUTES: f64 = 10.0;

pub fn met(kind: ExerciseKind, intensity: Intensity) -> f64 {
    use ExerciseKind::*;
    use Intensity::*;
    match (kind, intensity) {
        (Strength, Low) => 3.5,
        (Strength, Moderate) => 5.0,
        (Strength, High) => 6.0,
        (Cardio, Low) => 4.0,
        (Cardio, Moderate) => 7.0,
        (Cardio, High) => 10.0,
        (Mobility, Low) => 2.3,
        (Mobility, Moderate) => 2.8,
        (Mobility, High) => 4.0,
        (Sport, Low) => 4.5,
        (Sport, Moderate) => 7.0,
        (Sport, High) => 9.0,
        (Other, Low) => 3.0,
        (Other, Moderate) => 4.5,
        (Other, High) => 6.0,
    }
}

/// The exercise's own duration, else `sets × MINUTES_PER_SET`, else the default.
pub fn exercise_minutes(e: &Exercise) -> f64 {
    match (e.duration_min, e.sets) {
        (Some(d), _) => d,
        (None, Some(s)) => f64::from(s) * MINUTES_PER_SET,
        (None, None) => DEFAULT_EXERCISE_MINUTES,
    }
}

fn net_kcal(e: &Exercise, minutes: f64, weight_kg: f64) -> f64 {
    (met(e.kind, e.intensity) - 1.0) * weight_kg * minutes / 60.0
}

/// The hardest exercise's intensity (`Low` for none).
pub fn session_intensity(exercises: &[Exercise]) -> Intensity {
    exercises
        .iter()
        .map(|e| e.intensity)
        .max()
        .unwrap_or(Intensity::Low)
}

/// A session's duration (min) and net energy (kcal), both rounded to whole numbers.
/// `stated_minutes`, when the user gave the session's total time, wins over the sum of the
/// exercises, and the energy is scaled to it. Energy is `None` without a body weight.
pub fn session_energy(
    exercises: &[Exercise],
    stated_minutes: Option<f64>,
    weight_kg: Option<f64>,
) -> (f64, Option<f64>) {
    let minutes: Vec<f64> = exercises.iter().map(exercise_minutes).collect();
    let summed: f64 = minutes.iter().sum();
    let total = stated_minutes.unwrap_or(summed);
    let scale = if summed > 0.0 { total / summed } else { 0.0 };
    let kcal = weight_kg.map(|kg| {
        exercises
            .iter()
            .zip(&minutes)
            .map(|(e, m)| net_kcal(e, m * scale, kg))
            .sum::<f64>()
            .round()
    });
    (total.round(), kcal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(kind: ExerciseKind, intensity: Intensity) -> Exercise {
        Exercise {
            name: "x".into(),
            kind,
            intensity,
            sets: None,
            reps: None,
            weight_kg: None,
            duration_min: None,
            distance_km: None,
            note: None,
        }
    }

    #[test]
    fn minutes_come_from_duration_then_sets_then_the_default() {
        let mut e = ex(ExerciseKind::Strength, Intensity::Moderate);
        assert_eq!(exercise_minutes(&e), DEFAULT_EXERCISE_MINUTES);
        e.sets = Some(4);
        assert_eq!(exercise_minutes(&e), 10.0);
        e.duration_min = Some(15.0);
        assert_eq!(exercise_minutes(&e), 15.0);
    }

    #[test]
    fn hand_computed_session_energy() {
        // 30 min moderate running at 60 kg: (7 − 1) × 60 × 0.5 = 180 kcal.
        // 4 sets moderate strength = 10 min: (5 − 1) × 60 × 10/60 = 40 kcal.
        let run = Exercise {
            duration_min: Some(30.0),
            ..ex(ExerciseKind::Cardio, Intensity::Moderate)
        };
        let squat = Exercise {
            sets: Some(4),
            ..ex(ExerciseKind::Strength, Intensity::Moderate)
        };
        let exercises = [run, squat];
        assert_eq!(
            session_energy(&exercises, None, Some(60.0)),
            (40.0, Some(220.0))
        );
        // A stated total of 60 min scales every exercise by 1.5.
        assert_eq!(
            session_energy(&exercises, Some(60.0), Some(60.0)),
            (60.0, Some(330.0))
        );
        assert_eq!(session_energy(&exercises, None, None), (40.0, None));
    }

    #[test]
    fn intensity_is_the_hardest_exercises() {
        assert_eq!(session_intensity(&[]), Intensity::Low);
        assert_eq!(
            session_intensity(&[
                ex(ExerciseKind::Mobility, Intensity::Low),
                ex(ExerciseKind::Cardio, Intensity::High),
            ]),
            Intensity::High
        );
    }
}
