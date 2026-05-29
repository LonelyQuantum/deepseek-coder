import type { AgentEventEnvelope } from "@prole-coder/protocol" with {
  "resolution-mode": "import",
};

export const CONTEXT_SOURCE_PREVIEW_LIMIT = 8;

export type ContextVizStatus = "empty" | "ready";
export type ContextPlacement = "stable_prefix" | "dynamic_prelude" | "turn_suffix";

export interface ContextTokenSegment {
  readonly placement: ContextPlacement;
  readonly label: string;
  readonly tokens: number;
  readonly percent: number;
  readonly itemCount: number;
}

export interface ContextSourceSummary {
  readonly kind: string;
  readonly label: string;
  readonly tokens: number;
  readonly required: boolean;
  readonly reason: string;
  readonly omissionReason?: string;
}

export interface ContextEstimatorSummary {
  readonly name: string;
  readonly exact: boolean;
  readonly description: string;
  readonly calibration?: {
    readonly sampleCount: number;
    readonly inputUnit: string;
    readonly slopePpm: number;
    readonly interceptTokens: number;
    readonly meanAbsolutePercentageErrorPpm: number;
  };
}

export interface ContextManifestSummary {
  readonly manifestHash: string;
  readonly maxEntries: number;
  readonly totalDiscoveredFiles: number;
  readonly includedFiles: number;
  readonly omitted: readonly {
    readonly reason: string;
    readonly count: number;
  }[];
}

export interface ContextVizSnapshot {
  readonly status: ContextVizStatus;
  readonly runId?: string;
  readonly turnId?: string;
  readonly seq?: number;
  readonly inputTokens: number;
  readonly maxInputTokens: number;
  readonly inputPercent: number;
  readonly stablePrefixHash?: string;
  readonly stablePrefixBudgetTokens: number;
  readonly stablePrefixBudgetPercent: number;
  readonly stablePrefixBudgetRatioPercent: number;
  readonly cacheHitTokens?: number;
  readonly cacheMissTokens?: number;
  readonly estimator?: ContextEstimatorSummary;
  readonly segments: readonly ContextTokenSegment[];
  readonly includedSourceCount: number;
  readonly omittedSourceCount: number;
  readonly includedSources: readonly ContextSourceSummary[];
  readonly omittedSources: readonly ContextSourceSummary[];
  readonly manifest?: ContextManifestSummary;
}

export function emptyContextViz(): ContextVizSnapshot {
  return {
    status: "empty",
    inputTokens: 0,
    maxInputTokens: 0,
    inputPercent: 0,
    stablePrefixBudgetTokens: 0,
    stablePrefixBudgetPercent: 0,
    stablePrefixBudgetRatioPercent: 0,
    segments: [],
    includedSourceCount: 0,
    omittedSourceCount: 0,
    includedSources: [],
    omittedSources: [],
  };
}

export function contextVizFromEvent(event: AgentEventEnvelope): ContextVizSnapshot | undefined {
  if (event.type !== "context.built") {
    return undefined;
  }

  const payload = record(event.payload);
  if (payload === undefined) {
    return undefined;
  }

  const inputTokens = numberField(payload, "inputTokens");
  const maxInputTokens = numberField(payload, "maxInputTokens");
  const stablePrefixTokens = numberField(payload, "stablePrefixTokens");
  const stablePrefixBudgetTokens = numberField(payload, "stablePrefixBudgetTokens");
  const stablePrefixBudgetRatioPpm = numberField(payload, "stablePrefixBudgetRatioPpm");
  const dynamicPreludeTokens = numberField(payload, "dynamicPreludeTokens");
  const turnSuffixTokens = numberField(payload, "turnSuffixTokens");
  if (
    inputTokens === undefined ||
    maxInputTokens === undefined ||
    stablePrefixTokens === undefined ||
    stablePrefixBudgetTokens === undefined ||
    stablePrefixBudgetRatioPpm === undefined ||
    dynamicPreludeTokens === undefined ||
    turnSuffixTokens === undefined
  ) {
    return undefined;
  }

  const sectionItems = sectionItemCounts(payload["sections"]);
  const included = sourceSummaries(payload["includedSources"], false);
  const omitted = sourceSummaries(payload["omittedSources"], true);
  const stablePrefixHash = textField(payload, "stablePrefixHash");
  const cacheHitTokens = numberField(payload, "cacheHitTokens");
  const cacheMissTokens = numberField(payload, "cacheMissTokens");
  const estimator = estimatorSummary(payload["estimator"]);
  const manifest = manifestSummary(payload["manifest"]);
  const segments = tokenSegments(
    stablePrefixTokens,
    dynamicPreludeTokens,
    turnSuffixTokens,
    sectionItems,
  );

  return {
    status: "ready",
    runId: event.runId,
    ...(event.turnId === undefined ? {} : { turnId: event.turnId }),
    seq: event.seq,
    inputTokens,
    maxInputTokens,
    inputPercent: percentOf(inputTokens, maxInputTokens),
    ...(stablePrefixHash === undefined ? {} : { stablePrefixHash }),
    stablePrefixBudgetTokens,
    stablePrefixBudgetPercent: percentOf(stablePrefixTokens, stablePrefixBudgetTokens),
    stablePrefixBudgetRatioPercent: ratioPpmToPercent(stablePrefixBudgetRatioPpm),
    ...(cacheHitTokens === undefined ? {} : { cacheHitTokens }),
    ...(cacheMissTokens === undefined ? {} : { cacheMissTokens }),
    ...(estimator === undefined ? {} : { estimator }),
    segments,
    includedSourceCount: included.length,
    omittedSourceCount: omitted.length,
    includedSources: topSources(included),
    omittedSources: topSources(omitted),
    ...(manifest === undefined ? {} : { manifest }),
  };
}

function tokenSegments(
  stablePrefixTokens: number,
  dynamicPreludeTokens: number,
  turnSuffixTokens: number,
  itemCounts: ReadonlyMap<ContextPlacement, number>,
): readonly ContextTokenSegment[] {
  const total = stablePrefixTokens + dynamicPreludeTokens + turnSuffixTokens;
  return [
    segment("stable_prefix", "StablePrefix", stablePrefixTokens, total, itemCounts),
    segment("dynamic_prelude", "DynamicPrelude", dynamicPreludeTokens, total, itemCounts),
    segment("turn_suffix", "TurnSuffix", turnSuffixTokens, total, itemCounts),
  ];
}

function segment(
  placement: ContextPlacement,
  label: string,
  tokens: number,
  total: number,
  itemCounts: ReadonlyMap<ContextPlacement, number>,
): ContextTokenSegment {
  return {
    placement,
    label,
    tokens,
    percent: percentOf(tokens, total),
    itemCount: itemCounts.get(placement) ?? 0,
  };
}

function sectionItemCounts(value: unknown): ReadonlyMap<ContextPlacement, number> {
  const counts = new Map<ContextPlacement, number>();
  if (!Array.isArray(value)) {
    return counts;
  }

  for (const entry of value) {
    const section = record(entry);
    if (section === undefined) {
      continue;
    }

    const placement = placementField(section, "placement");
    const itemCount = numberField(section, "itemCount");
    if (placement !== undefined && itemCount !== undefined) {
      counts.set(placement, itemCount);
    }
  }

  return counts;
}

function sourceSummaries(value: unknown, omitted: boolean): ContextSourceSummary[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const summaries: ContextSourceSummary[] = [];
  for (const entry of value) {
    const source = record(entry);
    if (source === undefined) {
      continue;
    }

    const kind = textField(source, "kind");
    const required = booleanField(source, "required");
    const tokens = numberField(source, omitted ? "estimatedTokens" : "tokens");
    const reason = textField(source, omitted ? "inclusionReason" : "reason");
    if (kind === undefined || required === undefined || tokens === undefined || reason === undefined) {
      continue;
    }

    const omissionReason = omitted ? textField(source, "omissionReason") : undefined;
    summaries.push({
      kind,
      label: sourceLabel(source, kind),
      tokens,
      required,
      reason,
      ...(omissionReason === undefined ? {} : { omissionReason }),
    });
  }

  return summaries;
}

function topSources(sources: readonly ContextSourceSummary[]): readonly ContextSourceSummary[] {
  return sources
    .map((source, index) => ({ source, index }))
    .sort((left, right) => right.source.tokens - left.source.tokens || left.index - right.index)
    .slice(0, CONTEXT_SOURCE_PREVIEW_LIMIT)
    .map((entry) => entry.source);
}

function sourceLabel(source: Record<string, unknown>, kind: string): string {
  return (
    textField(source, "path") ??
    textField(source, "title") ??
    textField(source, "commandId") ??
    kind
  );
}

function estimatorSummary(value: unknown): ContextEstimatorSummary | undefined {
  const estimator = record(value);
  if (estimator === undefined) {
    return undefined;
  }

  const name = textField(estimator, "name");
  const exact = booleanField(estimator, "exact");
  const description = textField(estimator, "description");
  if (name === undefined || exact === undefined || description === undefined) {
    return undefined;
  }

  const calibration = calibrationSummary(estimator["calibration"]);
  return {
    name,
    exact,
    description,
    ...(calibration === undefined ? {} : { calibration }),
  };
}

function calibrationSummary(value: unknown): ContextEstimatorSummary["calibration"] | undefined {
  const calibration = record(value);
  if (calibration === undefined) {
    return undefined;
  }

  const sampleCount = numberField(calibration, "sampleCount");
  const inputUnit = textField(calibration, "inputUnit");
  const slopePpm = numberField(calibration, "slopePpm");
  const interceptTokens = finiteNumberField(calibration, "interceptTokens");
  const meanAbsolutePercentageErrorPpm = numberField(
    calibration,
    "meanAbsolutePercentageErrorPpm",
  );
  if (
    sampleCount === undefined ||
    inputUnit === undefined ||
    slopePpm === undefined ||
    interceptTokens === undefined ||
    meanAbsolutePercentageErrorPpm === undefined
  ) {
    return undefined;
  }

  return {
    sampleCount,
    inputUnit,
    slopePpm,
    interceptTokens,
    meanAbsolutePercentageErrorPpm,
  };
}

function manifestSummary(value: unknown): ContextManifestSummary | undefined {
  const manifest = record(value);
  if (manifest === undefined) {
    return undefined;
  }

  const manifestHash = textField(manifest, "manifestHash");
  const maxEntries = numberField(manifest, "maxEntries");
  const totalDiscoveredFiles = numberField(manifest, "totalDiscoveredFiles");
  const includedFiles = numberField(manifest, "includedFiles");
  if (
    manifestHash === undefined ||
    maxEntries === undefined ||
    totalDiscoveredFiles === undefined ||
    includedFiles === undefined
  ) {
    return undefined;
  }

  return {
    manifestHash,
    maxEntries,
    totalDiscoveredFiles,
    includedFiles,
    omitted: manifestOmitted(manifest["omitted"]),
  };
}

function manifestOmitted(value: unknown): ContextManifestSummary["omitted"] {
  if (!Array.isArray(value)) {
    return [];
  }

  const omitted: Array<{ readonly reason: string; readonly count: number }> = [];
  for (const entry of value) {
    const item = record(entry);
    const reason = textField(item, "reason");
    const count = numberField(item, "count");
    if (reason !== undefined && count !== undefined) {
      omitted.push({ reason, count });
    }
  }
  return omitted;
}

function placementField(
  payload: Record<string, unknown> | undefined,
  key: string,
): ContextPlacement | undefined {
  const value = textField(payload, key);
  return value === "stable_prefix" || value === "dynamic_prelude" || value === "turn_suffix"
    ? value
    : undefined;
}

function textField(payload: Record<string, unknown> | undefined, key: string): string | undefined {
  const value = payload?.[key];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function numberField(payload: Record<string, unknown> | undefined, key: string): number | undefined {
  const value = payload?.[key];
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : undefined;
}

function finiteNumberField(
  payload: Record<string, unknown> | undefined,
  key: string,
): number | undefined {
  const value = payload?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function booleanField(payload: Record<string, unknown> | undefined, key: string): boolean | undefined {
  const value = payload?.[key];
  return typeof value === "boolean" ? value : undefined;
}

function record(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function percentOf(value: number, total: number): number {
  return total > 0 ? Math.round((value / total) * 1000) / 10 : 0;
}

function ratioPpmToPercent(value: number): number {
  return Math.round(value / 1000) / 10;
}
