export interface ModelConfig {
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
}

export interface ModelConfigInput {
  baseUrl: string;
  model: string;
  apiKey?: string | null;
}

export interface SelectionRect {
  monitorId: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
}

export interface GradeInput {
  selection: SelectionRect;
  rubric: string;
  referenceEssay: string;
  maxScore: number;
  mode: "single" | "auto";
}

export interface GradeResult {
  score: number;
  maxScore: number;
  level: string;
  extractedText: string;
  breakdown: unknown;
  comments: string;
  suggestions: string;
  raw: string;
}
