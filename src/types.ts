export interface ModelConfig {
  baseUrl: string;
  model: string;
  memoryKey: string;
  hasApiKey: boolean;
}

export interface ModelConfigInput {
  baseUrl: string;
  model: string;
  memoryKey: string;
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
  recordId: string;
  recordPath: string;
  timingsMs: Record<string, number>;
  score: number;
  maxScore: number;
  level: string;
  extractedText: string;
  breakdown: unknown;
  comments: string;
  suggestions: string;
  raw: string;
}

export interface CorrectionInput {
  selection: SelectionRect;
  result: GradeResult;
  correctedScore: number;
  correctionNote: string;
  rubric: string;
  referenceEssay: string;
  maxScore: number;
}

export interface MemoryWikiSaveResult {
  path: string;
  entry: string;
}

export interface GradeRecord {
  id: string;
  timestamp: number;
  memoryKey: string;
  status: "success" | "error" | "correction";
  mode: string;
  model: string;
  maxScore: number;
  score?: number;
  level?: string;
  error?: string;
  timingsMs: Record<string, number>;
  capturePath?: string;
  recordPath?: string;
  extractedText?: string;
  comments?: string;
}
