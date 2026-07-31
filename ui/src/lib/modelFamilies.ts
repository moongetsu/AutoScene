/** InsightFace (and similar) pack families: UI family + optional size level → wire model id. */

export type ModelLevel = {
  id: string
  label: string
  recommended?: boolean
}

export type ModelFamily = {
  id: string
  label: string
  /** Full wire ids for this family, ordered High → Low when tiered */
  levels: ModelLevel[]
  defaultLevelId: string
}

export const INSIGHTFACE_FAMILIES: ModelFamily[] = [
  {
    id: "buffalo",
    label: "Everyday match",
    defaultLevelId: "buffalo_l",
    levels: [
      { id: "buffalo_l", label: "High", recommended: true },
      { id: "buffalo_m", label: "Medium" },
      { id: "buffalo_s", label: "Low" },
    ],
  },
  {
    id: "antelopev2",
    label: "Precision match",
    defaultLevelId: "antelopev2",
    levels: [{ id: "antelopev2", label: "Default" }],
  },
]

export function findFamilyByModelId(
  families: ModelFamily[],
  modelId: string,
): ModelFamily | undefined {
  return families.find((f) => f.levels.some((l) => l.id === modelId))
}

export function familyHasLevels(family: ModelFamily): boolean {
  return family.levels.length > 1
}

/** DeepFace Compare-mode pack ids (wire values for Quality pack select). */
export const DEEPFACE_MODEL_IDS = ["Facenet512", "ArcFace", "Buffalo_L"] as const
export type DeepFaceModelId = (typeof DEEPFACE_MODEL_IDS)[number]

export function resolveDeepfaceModel(modelId: string | undefined | null): DeepFaceModelId {
  if (modelId && (DEEPFACE_MODEL_IDS as readonly string[]).includes(modelId)) {
    return modelId as DeepFaceModelId
  }
  return "Facenet512"
}

export function properCaseWireLabel(id: string): string {
  if (id === "Facenet512") return "Facenet512"
  if (id === "ArcFace") return "ArcFace"
  if (id === "Buffalo_L") return "Large"
  return id
}
