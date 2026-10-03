import type { Exercise } from "../api/types";
import { strings } from "../i18n";

const t = strings.training.exercise;

/** "深蹲 4×8 60 kg" / "Run 30 min 5 km": the exercise's own numbers, worded per locale. */
export function exerciseLabel(e: Exercise): string {
  const parts = [e.name];
  if (e.sets != null && e.reps != null) parts.push(t.setsReps(e.sets, e.reps));
  else if (e.sets != null) parts.push(t.sets(e.sets));
  else if (e.reps != null) parts.push(t.reps(e.reps));
  if (e.weight_kg != null && e.weight_kg > 0) parts.push(t.weight(e.weight_kg));
  if (e.duration_min != null) parts.push(t.duration(e.duration_min));
  if (e.distance_km != null) parts.push(t.distance(e.distance_km));
  return parts.join(" ");
}
