export type ProjectStatus = 'draft' | 'script' | 'storyboard' | 'editing' | 'exported'
export type AspectRatio = '16:9' | '9:16'
export type TtsProvider = 'gemini' | 'elevenlabs' | 'speko'
export type LlmProvider = 'gemini' | 'openrouter' | 'groq' | 'custom'
export type ApiProvider = 'speko' | 'higgsfield' | 'gemini' | 'elevenlabs' | 'openrouter' | 'groq' | 'custom' | 'flow2api'
/** Where a live model list comes from: an LLM provider, or a TTS catalog. */
export type ModelSource = LlmProvider | 'gemini-tts' | 'elevenlabs' | 'speko-tts' | 'speko-stt'
export type MotionType = 'camera' | 'video'
export type CameraPresetId = 'zoomin' | 'zoomout' | 'panleft' | 'panright' | 'kenburns' | 'shake' | 'static'
export type MotionStrength = 'halus' | 'sedang' | 'kuat'
export type TransitionId = 'cut' | 'fade' | 'slideleft' | 'zoomin'
export type CaptionStyleId = 'karaoke' | 'tebal' | 'minimal' | 'subtitle' | 'none'
export type CaptionPosition = 'atas' | 'tengah' | 'bawah'
export type CaptionSize = 'kecil' | 'sedang' | 'besar'
/** 1 idea, 2 script, 3 storyboard, 4 editor */
export type Step = 1 | 2 | 3 | 4

export interface YoutubeMetadata { description: string; tags: string[]; tagsText?: string; generatedAt: number }

export interface EditorSettings {
  youtubeMetadata?: YoutubeMetadata
  captionOffsetSec?: number
  captionAnchor?: { x: number; y: number } | null
  captionEdits?: Record<string, { audioAssetId: string; words: WordTiming[] }>
  captionStyle: CaptionStyleId
  captionPosition: CaptionPosition
  captionSize: CaptionSize
  /** Caption size as a multiple of the style's size (0.5–1.8); overrides captionSize when set. */
  captionScale?: number
  /** Custom look on top of the chosen caption style; unset fields keep the style's own. */
  captionFont?: string
  captionColor?: string
  captionHighlight?: string
  /** Box behind the caption: a color and opacity, or null for no box. */
  captionBg?: { color: string; opacity: number } | null
  musicAssetId: string | null
  musicVolumeDb: number
  duckMusic: boolean
  /** Logos, watermarks and text placed over the video. */
  overlays: Overlay[]
  /** Last prompt written for making background music in an outside AI (Suno, Udio, …). */
  musicPrompt?: MusicPrompt | null
}

export interface MusicPrompt {
  /** Suno "Style of Music": comma-separated English tags. */
  style: string
  /** Suno "Exclude styles". */
  exclude: string
  title: string
  /** One paragraph for other music AIs (Udio, ElevenLabs Music, Stable Audio, …). */
  description: string
  /** Why this music fits, in Indonesian, for the user. */
  reason: string
  /** Video length when it was written, in seconds. */
  durationSec: number
}

/**
 * Where an overlay sits: `anchor` is a numpad position on the overlay itself (7 top-left, 5 center,
 * 3 bottom-right, the same numbering as ASS n), and `x`/`y` place that point as a fraction of the frame.
 */
interface OverlayBase {
  id: string
  anchor: 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9
  x: number
  y: number
  /** Seconds on the final timeline; `end` null means until the video ends. */
  start: number
  end: number | null
}

export interface ImageOverlay extends OverlayBase {
  kind: 'image'
  assetId: string
  /** Width as a fraction of the frame width. */
  width: number
  opacity: number
}

export type TextOverlayStyle = 'plain' | 'outline' | 'box'

export interface TextOverlay extends OverlayBase {
  kind: 'text'
  text: string
  /** Font size as a fraction of the frame height. */
  size: number
  color: string
  style: TextOverlayStyle
  bold: boolean
}

export type Overlay = ImageOverlay | TextOverlay

export interface Project {
  id: string
  title: string
  synopsis: string
  durationSec: number
  language: string
  aspectRatio: AspectRatio
  styleId: string
  ttsProvider: TtsProvider
  ttsVoice: string
  /** Higgsfield endpoint ids; null falls back to the app default. */
  imageModel: string | null
  videoModel: string | null
  /** One of the video model's resolutions, e.g. "720p"; null or unsupported uses the default (1080p when offered). */
  videoResolution: string | null
  status: ProjectStatus
  step: Step
  coverAssetId: string | null
  editor: EditorSettings
  createdAt: number
  updatedAt: number
}

export interface Character {
  id: string
  projectId: string
  name: string
  description: string
  sheetAssetId: string | null
  sort: number
}

export interface Clip {
  id: string
  projectId: string
  sort: number
  title: string
  /** What happens in the scene, in the script's language (the Naskah step). */
  story: string
  visualPrompt: string
  /** The story text the current visual plan was written from; null before any plan. */
  visualSource: string | null
  narration: string
  durationMs: number
  characterIds: string[]
  motionType: MotionType
  cameraPreset: CameraPresetId
  motionStrength: MotionStrength
  videoPrompt: string
  transition: TransitionId
  imageAssetId: string | null
  videoAssetId: string | null
  audioAssetId: string | null
}

export type AssetKind = 'image' | 'video' | 'audio' | 'music' | 'overlay' | 'export' | 'thumbnail'

export interface WordTiming {
  text: string
  start: number
  end: number
}

export interface AssetMeta {
  words?: WordTiming[]
  estimatedTimings?: boolean
  voice?: string
  [key: string]: unknown
}

export interface Asset {
  id: string
  projectId: string
  clipId: string | null
  characterId: string | null
  kind: AssetKind
  provider: string | null
  model: string | null
  prompt: string | null
  remoteId: string | null
  remoteUrl: string | null
  localPath: string
  url: string
  durationMs: number | null
  width: number | null
  height: number | null
  meta: AssetMeta
  createdAt: number
}

/** "narration" voices several scenes in one take and cuts it per scene. */
export type JobKind = 'story' | 'image' | 'sheet' | 'video' | 'tts' | 'narration' | 'align' | 'export' | 'thumbnail'
export type JobStatus = 'queued' | 'running' | 'done' | 'failed' | 'canceled'

export interface Job {
  exportOptions?: ExportOptions | null
  id: string
  projectId: string
  clipId: string | null
  characterId: string | null
  kind: JobKind
  provider: string
  status: JobStatus
  progress: number
  message: string | null
  error: string | null
  resultAssetId: string | null
  createdAt: number
  updatedAt: number
}

export interface ProjectBundle {
  project: Project
  characters: Character[]
  clips: Clip[]
  assets: Asset[]
  jobs: Job[]
}

export interface ProjectSummary {
  id: string
  title: string
  styleId: string
  status: ProjectStatus
  step: Step
  clipCount: number
  totalMs: number
  coverUrl: string | null
  updatedAt: number
}

/** Existing asset links stay native-owned; links on restored rows are validated during insertion. */
export interface ProjectSnapshot {
  project: Omit<Project, 'createdAt' | 'updatedAt' | 'coverAssetId'>
  characters: Character[]
  clips: Clip[]
}

export interface NewProjectInput {
  synopsis?: string
  durationSec?: number
  language?: string
  aspectRatio?: AspectRatio
  styleId?: string
  ttsProvider?: TtsProvider
  ttsVoice?: string
}

export interface KeyStatus {
  provider: ApiProvider
  configured: boolean
  lastOk: boolean | null
  lastMessage: string | null
  checkedAt: number | null
  /** Masked key for display, e.g. "sk-or-v1…9f3c". Null when there is no readable key. */
  preview: string | null
  /** A key is stored but can no longer be decrypted and must be entered again. */
  unreadable: boolean
  /** How many keys are stored. Gemini keeps a pool; every other provider holds 0 or 1. */
  keyCount: number
}

export interface KeyTestResult {
  ok: boolean
  message: string
}

export interface HiggsfieldMcpStatus {
  configured: boolean
  connected: boolean
  email: string | null
  credits: number | null
  unlimited: boolean
  expiresAt: string | null
  error: string | null
}

/** Result of testing one key out of a provider's stored pool, e.g. one Gemini key. */
export interface GeminiKeyTestResult {
  /** 1-based position in the saved pool, so results can be shown in order. */
  index: number
  /** Masked key, never the raw value. */
  preview: string
  ok: boolean
  message: string | null
}

export interface VoiceOption {
  id: string
  name: string
  description: string
  previewUrl?: string | null
}

export interface AppSettings {
  theme: 'dark' | 'light'
  captionEngine: 'whisper' | 'speko'
  spekoTtsModel: string
  spekoSttModel: string
  defaultTtsProvider: TtsProvider
  defaultLanguage: string
  exportFolder: string | null
  exportTarget: 'local' | 'ssh'
  exportSshProfile: string
  sshConnections?: SshConnection[]
  whisperSshProfile?: string
  sshConnection?: SshConnection
  /** Which LLM writes stories and narration. */
  llmProvider: LlmProvider
  /** Chosen model per LLM provider, so switching providers keeps each choice. */
  llmModels: Record<LlmProvider, string>
  /** Base URL of an OpenAI-compatible endpoint, e.g. http://localhost:11434/v1 */
  customBaseUrl: string
  /** Base URL of the Flow2API server; only used in `external` mode. */
  flow2apiBaseUrl: string
  /** `bundled` runs the Flow2API server shipped with the app; `external` talks to one the user runs. */
  flow2apiMode: 'bundled' | 'external'
  flow2apiParallelAccounts: number
  /** Opt-in loopback port for the extension worker; 0 means a random port each launch. */
  flow2apiFixedPort: number
  geminiTtsModel: string
  elevenModel: string
  /** Higgsfield models for new projects: the last ones picked in the Idea step. */
  imageModel: string
  videoModel: string
  /** Automatically align Gemini narration using the selected caption engine; keep legacy setting name for existing projects. */
  whisperAuto: boolean
  whisperTarget: 'local' | 'ssh'
  vidgenParallelAccounts: number
}

export interface VidgenAccount {
  id: string
  label: string
  enabled: boolean
  configured: boolean
  info: { ok: boolean; error?: string; checkedAt?: number; user?: { uid?: string; email?: string; credits?: number }; unlimited?: { active?: boolean; daysLeft?: number; expiresAt?: string }; limits?: { dailyVideosRemaining?: number; currentActiveJobs?: number; maxConcurrentJobs?: number } } | null
}

export interface WhisperRemoteStatus {
  ready: boolean
  model?: boolean
  cli?: boolean
  cpus?: number
  freeGb?: number
}

/** What one Higgsfield request would cost, from the API's estimate endpoint. */
export interface CreditEstimate {
  credits: number
  usd: number | null
}

/** The local Whisper model used to time caption words. */
export interface WhisperStatus {
  state: 'missing' | 'downloading' | 'verifying' | 'ready'
  received: number
  total: number
  error: string | null
}

/** The Flow2API backend the app runs or talks to. */
export interface Flow2ApiStatus {
  state: 'starting' | 'restarting' | 'ready' | 'failed' | 'stopped'
  baseUrl: string | null
  version: string | null
  message: string | null
}

/** Credentials for the bundled Flow2API admin panel. */
export interface Flow2ApiPanelInfo {
  baseUrl: string
  loginUrl: string
  username: string
  password: string
}

/** Where the optional Chrome extension worker connects. */
export interface Flow2ApiExtensionInfo {
  wsUrl: string
  key: string | null
  extensionDir: string | null
  fixedPort: boolean
}

export interface ModelOption {
  id: string
  name: string
  description?: string
  contextLength?: number | null
  /** USD per 1M input / output tokens, when the provider publishes it. */
  priceIn?: number | null
  priceOut?: number | null
  /** Short labels the picker can filter on, e.g. "JSON", "Gratis", "Gambar". */
  tags: string[]
  /** Model family, e.g. "Kling", for a brand mark next to the name. */
  family?: string
  /** Higgsfield credits (and dollars) for one request; undefined while unknown, null when it cannot be estimated. */
  credits?: number | null
  usd?: number | null
  /** What the credits pay for, e.g. "10 dtk". */
  creditUnit?: string
}

export interface ModelList {
  models: ModelOption[]
  fetchedAt: number
}

export interface SshConnection {
  authMethod?: 'key' | 'password'
  passwordConfigured?: boolean
  profile: string
  host: string
  port: number
  username: string
  identityFile: string
}

export interface SshStatus {
  cpus: number
  freeGb: number
  totalGb: number
  ffmpeg: string | null
  worker: boolean
}

export interface SshResult {
  id: string
  title: string
  filename: string
  createdAt: number
  state: 'completed' | 'running' | 'failed' | 'incomplete'
  bytes: number
  error?: string
}

export interface SshResults {
  storage: { freeGb: number; usedGb: number; totalGb: number }
  results: SshResult[]
}

export interface ExportOptions {
  target: 'local' | 'ssh'
  sshProfile: string
  deleteAfterDownload: boolean
  fileName: string
  resolution: 720 | 1080 | 2160
  fps: 30 | 60
  burnCaptions: boolean
  writeSrt: boolean
  normalizeAudio: boolean
  duckMusic: boolean
  folder: string
}

export interface ClipPatchEvent {
  clipId: string
  patch: Partial<Clip>
}

export interface CharacterPatchEvent {
  characterId: string
  patch: Partial<Character>
}

export interface FlowSessionProfile {
  id: string; tokenId: number | null; email: string; state: string; lastSync: number | null; error: string; browserOpen: boolean; busy: boolean; active: boolean
}
export interface FlowSessionsState {
  profiles: FlowSessionProfile[]
  accounts: {tokenId:number;email:string;active:boolean;managed:boolean;lastSync:string|null}[]
  autoRefresh:boolean
  intervalMinutes:number
  bulkRunning:boolean
}
