import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ArrowRight, Brush, Check, ChevronDown, CircleHelp, Download, Eraser, FileImage, Film, FolderOpen, Image as ImageIcon, Maximize2, Minimize2, Pause, Play, RotateCcw, Settings2, Trash2, Undo2, UploadCloud, Volume2, VolumeX, WandSparkles, X, Zap } from "lucide-react";
import rotoNowLogo from "./assets/roto-now-logo.png";

type MediaKind = "image" | "video";
type Quality = "Fast" | "Balanced" | "Maximum";
type Model = "General" | "Anime";
type VideoModel = "General Lite" | "General Maximum" | "Anime";
type ModelId = "generalLite" | "general" | "anime";
type ScreenColor = "green" | "blue";
type TrackingMode = "temporal" | "cutie";
type CutieTier = "balanced" | "high";
type PreviewMode = "input" | "output";
type CorrectionMode = "restore" | "erase";

interface ImportedMedia { file?: File; path?: string; name: string; size: number; kind: MediaKind; url: string; }
interface NativeMediaInfo { path: string; name: string; size: number; kind: MediaKind; previewDataUrl?: string; }
interface PerformanceMetrics { decodeMs: number; preprocessMs: number; inferenceMs: number; postprocessMs: number; temporalAndCompositeMs: number; encodeMs: number; firstInferenceMs?: number; }
interface ProcessResult { outputPath: string; model: string; provider: string; precision: string; pipeline: string; performance?: PerformanceMetrics; durationMs: number; frameCount?: number; width?: number; height?: number; frameRate?: number; mediaDurationSeconds?: number; hasAudio?: boolean; preview: boolean; seed: boolean; sourceFramePath?: string; }
interface ModelStatus { id: ModelId; name: string; size: number; installed: boolean; managed: boolean; state: string; provider: string; }
interface CutieStatus { id: string; tier: CutieTier; name: string; size: number; width: number; height: number; installed: boolean; managed: boolean; state: string; provider: string; }
interface BootstrapStatus { ready: boolean; provider: string; models: ModelStatus[]; trackers: CutieStatus[]; }
interface EngineStatus { application: string; version: string; inferenceEngine: string; ffmpeg: string; }
interface JobProgress { phase: string; completed?: number; total?: number; percent?: number; etaSeconds?: number; message: string; }
interface CorrectionPoint { x: number; y: number; }
interface CorrectionStroke { mode: CorrectionMode; radius: number; points: CorrectionPoint[]; }
type JobEvent =
  | ({ jobId: string; type: "progress" } & JobProgress)
  | { jobId: string; type: "completed"; result: ProcessResult }
  | { jobId: string; type: "failed"; error: string }
  | { jobId: string; type: "cancelled" };

const isTauriRuntime = () => "__TAURI_INTERNALS__" in window;
const formatBytes = (bytes: number) => bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / (1024 * 1024)).toFixed(bytes > 1024 ** 3 ? 2 : 0)} MB`;
const isSupported = (file: File) => file.type.startsWith("image/") || file.type.startsWith("video/");
const formatTime = (seconds: number) => {
  const safe = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  const minutes = Math.floor(safe / 60);
  const remainder = Math.floor(safe % 60);
  return `${minutes}:${remainder.toString().padStart(2, "0")}`;
};

const VIDEO_MODEL_OPTIONS: ReadonlyArray<{ value: VideoModel; description: string }> = [
  { value: "General Lite", description: "Fastest general-purpose model" },
  { value: "General Maximum", description: "Higher detail, slower inference" },
  { value: "Anime", description: "Animation and line art" },
];

function VideoModelSelect({ value, onChange, ariaLabel }: { value: VideoModel; onChange: (value: VideoModel) => void; ariaLabel: string }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { setOpen(false); triggerRef.current?.focus(); }
    };
    document.addEventListener("pointerdown", closeOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  return <div className={`model-select ${open ? "open" : ""}`} ref={rootRef}>
    <button ref={triggerRef} type="button" className="model-select-trigger" aria-label={ariaLabel} aria-haspopup="listbox" aria-expanded={open} onClick={() => setOpen((current) => !current)} onKeyDown={(event) => { if (event.key === "ArrowDown") { event.preventDefault(); setOpen(true); } }}>
      <ImageIcon size={16} aria-hidden="true" /><span>{value}</span><ChevronDown size={15} aria-hidden="true" />
    </button>
    {open && <div className="model-select-menu" role="listbox" aria-label={ariaLabel}>
      {VIDEO_MODEL_OPTIONS.map((option) => <button key={option.value} type="button" className={`model-select-option ${option.value === value ? "selected" : ""}`} role="option" aria-selected={option.value === value} onClick={() => { if (option.value !== value) onChange(option.value); setOpen(false); triggerRef.current?.focus(); }}>
        <span><strong>{option.value}</strong><small>{option.description}</small></span>{option.value === value && <Check size={14} aria-hidden="true" />}
      </button>)}
    </div>}
  </div>;
}

function App() {
  const inputRef = useRef<HTMLInputElement>(null);
  const playheadRef = useRef(0);
  const requestedPreviewTimeRef = useRef(0);
  const [playhead, setPlayhead] = useState(0);
  const [media, setMedia] = useState<ImportedMedia | null>(null);
  const [dragging, setDragging] = useState(false);
  const [model, setModel] = useState<Model>("General");
  const [quality, setQuality] = useState<Quality>("Balanced");
  const [videoModel, setVideoModel] = useState<VideoModel>("General Lite");
  const [screenColor, setScreenColor] = useState<ScreenColor>("green");
  const [trackingMode, setTrackingMode] = useState<TrackingMode>("temporal");
  const [cutieTier, setCutieTier] = useState<CutieTier>("balanced");
  const [edgeDetail, setEdgeDetail] = useState(72);
  const [status, setStatus] = useState<"idle" | "ready" | "processing" | "done">("idle");
  const [error, setError] = useState<string | null>(null);
  const [fullResult, setFullResult] = useState<ProcessResult | null>(null);
  const [previewResult, setPreviewResult] = useState<ProcessResult | null>(null);
  const [seedResult, setSeedResult] = useState<ProcessResult | null>(null);
  const [seedEditorOpen, setSeedEditorOpen] = useState(false);
  const [previewMode, setPreviewMode] = useState<PreviewMode>("input");
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [bootstrap, setBootstrap] = useState<BootstrapStatus | null>(null);
  const [showModels, setShowModels] = useState(false);
  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [progress, setProgress] = useState<JobProgress | null>(null);
  const [correctionOpen, setCorrectionOpen] = useState(false);
  const [correctionMode, setCorrectionMode] = useState<CorrectionMode>("restore");
  const [brushRadius, setBrushRadius] = useState(0.025);
  const [correctionStrokes, setCorrectionStrokes] = useState<CorrectionStroke[]>([]);
  const [applyingCorrections, setApplyingCorrections] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [engineStatus, setEngineStatus] = useState<EngineStatus | null>(null);

  useEffect(() => {
    const url = media?.url;
    return () => { if (url?.startsWith("blob:")) URL.revokeObjectURL(url); };
  }, [media?.url]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const preventContextMenu = (event: MouseEvent) => event.preventDefault();
    document.documentElement.classList.add("native-runtime");
    document.addEventListener("contextmenu", preventContextMenu);
    return () => {
      document.removeEventListener("contextmenu", preventContextMenu);
      document.documentElement.classList.remove("native-runtime");
    };
  }, []);

  const updatePlaybackRef = useCallback((seconds: number) => {
    playheadRef.current = seconds;
  }, []);
  const updateSharedPlayhead = useCallback((seconds: number) => {
    playheadRef.current = seconds;
    setPlayhead(seconds);
  }, []);

  const refreshBootstrap = async () => {
    if (!isTauriRuntime()) { setBootstrap({ ready: true, provider: "Browser mock", models: [], trackers: [{ id: "cutieBalanced", tier: "balanced", name: "Cutie Balanced", size: 0, width: 640, height: 368, installed: false, managed: false, state: "unavailable", provider: "Native app only" }, { id: "cutieHigh", tier: "high", name: "Cutie High Detail", size: 0, width: 960, height: 544, installed: false, managed: false, state: "unavailable", provider: "Native app only" }] }); return; }
    setBootstrap(await invoke<BootstrapStatus>("get_bootstrap_status"));
  };

  useEffect(() => {
    void refreshBootstrap().catch((caught) => setError(String(caught)));
    if (isTauriRuntime()) void invoke<EngineStatus>("engine_status").then(setEngineStatus).catch(() => undefined);
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<JobEvent>("job-event", ({ payload }) => {
      if (disposed) return;
      if (payload.type === "progress") { setProgress(payload); return; }
      setActiveJobId(null);
      setProgress(null);
      if (payload.type === "failed") { setStatus((current) => current === "idle" ? "idle" : "ready"); setError(payload.error); return; }
      if (payload.type === "cancelled") { setStatus((current) => current === "idle" ? "idle" : "ready"); return; }
      if (!payload.result.outputPath) { void refreshBootstrap(); return; }
      if (payload.result.seed) {
        setSeedResult((current) => {
          if (current?.outputPath) void invoke("discard_output", { path: current.outputPath }).catch(() => undefined);
          if (current?.sourceFramePath) void invoke("discard_output", { path: current.sourceFramePath }).catch(() => undefined);
          return payload.result;
        });
        setCorrectionStrokes([]);
        setSeedEditorOpen(true);
        setStatus("ready");
        return;
      }
      if (payload.result.preview) {
        setPreviewResult(payload.result);
        playheadRef.current = requestedPreviewTimeRef.current;
        setPlayhead(requestedPreviewTimeRef.current);
      } else { setFullResult(payload.result); setCorrectionOpen(false); setCorrectionStrokes([]); }
      setPreviewMode("output");
      setStatus("done");
    }).then((cleanup) => { if (disposed) cleanup(); else unlisten = cleanup; }).catch((caught) => { if (!disposed) setError(String(caught)); });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  useEffect(() => {
    if (!helpOpen) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === "Escape") setHelpOpen(false); };
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("keydown", closeOnEscape);
      document.body.style.overflow = previousOverflow;
    };
  }, [helpOpen]);

  const discard = (result: ProcessResult | null) => {
    if (result?.outputPath && isTauriRuntime()) void invoke("discard_output", { path: result.outputPath }).catch(() => undefined);
  };
  const clearResults = () => { discard(fullResult); discard(previewResult); setFullResult(null); setPreviewResult(null); setPreviewMode("input"); setSavedPath(null); setCorrectionOpen(false); setCorrectionStrokes([]); };
  const clearSeed = () => {
    discard(seedResult);
    if (seedResult?.sourceFramePath && isTauriRuntime()) void invoke("discard_output", { path: seedResult.sourceFramePath }).catch(() => undefined);
    setSeedResult(null); setSeedEditorOpen(false); setCorrectionStrokes([]);
  };

  const acceptFile = (file?: File) => {
    if (!file) return;
    if (!isSupported(file)) { setError("Choose a supported image or video file."); return; }
    clearResults(); clearSeed();
    const kind: MediaKind = file.type.startsWith("video/") ? "video" : "image";
    setMedia({ file, name: file.name, size: file.size, kind, url: URL.createObjectURL(file) });
    setError(null); setStatus("ready"); playheadRef.current = 0; setPlayhead(0);
  };

  const acceptNativePath = async (path: string) => {
    const info = await invoke<NativeMediaInfo>("inspect_media", { path });
    if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
    clearResults(); clearSeed();
    setMedia({ path: info.path, name: info.name, size: info.size, kind: info.kind, url: info.previewDataUrl ?? convertFileSrc(info.path) });
    setError(null); setStatus("ready"); playheadRef.current = 0; setPlayhead(0);
  };

  const browseFiles = async () => {
    if (!isTauriRuntime()) { inputRef.current?.click(); return; }
    try {
      const selected = await openDialog({ multiple: false, directory: false, filters: [{ name: "Images and videos", extensions: ["png", "jpg", "jpeg", "webp", "mp4", "mov", "webm"] }] });
      if (!selected || Array.isArray(selected)) return;
      const info = await invoke<NativeMediaInfo>("inspect_media", { path: selected });
      clearResults(); clearSeed();
      setMedia({ path: info.path, name: info.name, size: info.size, kind: info.kind, url: info.previewDataUrl ?? convertFileSrc(info.path) });
      setError(null); setStatus("ready"); playheadRef.current = 0; setPlayhead(0);
    } catch (caught) { setError(String(caught)); }
  };

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (disposed) return;
      const canAcceptDrop = !media && !!bootstrap?.ready && !showModels && !helpOpen && status !== "processing" && !activeJobId;
      if (payload.type === "enter") { setDragging(canAcceptDrop); return; }
      if (payload.type === "leave") { setDragging(false); return; }
      if (payload.type !== "drop") return;
      setDragging(false);
      if (!canAcceptDrop) return;
      if (payload.paths.length !== 1) { setError("Drop one image or video at a time."); return; }
      void acceptNativePath(payload.paths[0]).catch((caught) => setError(String(caught)));
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    }).catch((caught) => {
      if (!disposed) setError(`Drag and drop is unavailable: ${String(caught)}`);
    });
    return () => { disposed = true; unlisten?.(); };
  }, [activeJobId, bootstrap?.ready, fullResult, helpOpen, media, previewResult, seedResult, showModels, status]);

  const reset = () => {
    clearResults(); clearSeed(); setMedia(null); setStatus("idle"); setError(null); playheadRef.current = 0; setPlayhead(0);
    if (inputRef.current) inputRef.current.value = "";
  };

  const requiredModelId = (_forPreview = false): ModelId => media?.kind === "video"
    ? videoModel === "Anime" ? "anime" : videoModel === "General Maximum" ? "general" : "generalLite"
    : model === "Anime" ? "anime" : quality === "Maximum" ? "general" : "generalLite";
  const requiredModel = (forPreview = false) => bootstrap?.models.find((item) => item.id === requiredModelId(forPreview));

  const downloadModel = async (modelId: ModelId) => {
    try { setError(null); setProgress({ phase: "downloading", message: "Starting model download" }); setActiveJobId(await invoke<string>("download_model", { modelId })); }
    catch (caught) { setProgress(null); setError(String(caught)); }
  };

  const removeModel = async (modelId: ModelId) => {
    try { await invoke("remove_model", { modelId }); await refreshBootstrap(); }
    catch (caught) { setError(String(caught)); }
  };

  const downloadCutie = async (tier: CutieTier) => {
    try { setError(null); setProgress({ phase: "downloading", message: "Starting Cutie download" }); setActiveJobId(await invoke<string>("download_cutie", { tier })); }
    catch (caught) { setProgress(null); setError(String(caught)); }
  };

  const removeCutie = async (tier: CutieTier) => {
    try { await invoke("remove_cutie", { tier }); await refreshBootstrap(); }
    catch (caught) { setError(String(caught)); }
  };

  const startSeedJob = async (customMaskPath?: string) => {
    if (!media?.path || media.kind !== "video" || status === "processing") return;
    const needed = requiredModel(false);
    if (!customMaskPath && needed && !needed.installed) { setError(`${needed.name} must be downloaded first.`); setShowModels(true); return; }
    if (!isTauriRuntime()) { setError("Primary Subject tracking is available in the native app."); return; }
    try {
      setError(null); setProgress({ phase: "starting", message: customMaskPath ? "Importing first-frame mask" : "Generating first-frame mask" }); setStatus("processing");
      setActiveJobId(await invoke<string>("start_video_seed_job", { inputPath: media.path, model: videoModel, quality: "Balanced", edgeDetail: 72, customMaskPath: customMaskPath ?? null }));
    } catch (caught) { setStatus("ready"); setProgress(null); setError(String(caught)); }
  };

  const attachSeedMask = async () => {
    if (!isTauriRuntime()) { setError("Attaching a subject mask is available in the native app."); return; }
    try {
      const selected = await openDialog({ multiple: false, directory: false, filters: [{ name: "First-frame subject mask", extensions: ["png"] }] });
      if (!selected || Array.isArray(selected)) return;
      await startSeedJob(selected);
    } catch (caught) { setError(String(caught)); }
  };

  const runJob = async (fastPreview = false) => {
    if (!media || status === "processing") return;
    const needed = requiredModel(fastPreview);
    const needsSegmentationModel = media.kind === "image" || fastPreview || trackingMode === "temporal";
    if (needsSegmentationModel && needed && !needed.installed) { setError(`${needed.name} must be downloaded first.`); setShowModels(true); return; }
    if (media.kind === "video" && !fastPreview && trackingMode === "cutie") {
      if (!bootstrap?.trackers.find((item) => item.tier === cutieTier)?.installed) { setError("The selected Cutie quality tier must be downloaded first."); setShowModels(true); return; }
      if (!seedResult) { setError("Create and confirm a first-frame subject mask before tracking."); return; }
    }
    if (!isTauriRuntime() || !media.path) { setStatus("processing"); window.setTimeout(() => setStatus("done"), 900); return; }
    try {
      setError(null); setSavedPath(null); setProgress({ phase: "starting", message: fastPreview ? "Preparing preview frame" : "Preparing job" }); setStatus("processing");
      if (fastPreview) { requestedPreviewTimeRef.current = playheadRef.current; discard(previewResult); setPreviewResult(null); }
      else { clearResults(); }
      const command = media.kind === "image" ? "start_image_job" : "start_video_job";
      const selectedModel = media.kind === "video" ? videoModel : model;
      const selectedQuality = media.kind === "video" ? "Balanced" : quality;
      const selectedEdgeDetail = media.kind === "video" && trackingMode === "cutie" ? 72 : edgeDetail;
      const id = await invoke<string>(command, {
        inputPath: media.path, model: selectedModel, quality: selectedQuality, edgeDetail: selectedEdgeDetail,
        ...(media.kind === "video" ? { screenColor, preview: fastPreview, startSeconds: fastPreview ? playheadRef.current : 0, trackingMode, seedMaskPath: !fastPreview && trackingMode === "cutie" ? seedResult?.outputPath : null, trackerQuality: cutieTier } : {}),
      });
      setActiveJobId(id);
    } catch (caught) { setStatus("ready"); setProgress(null); setError(String(caught)); }
  };

  const cancelJob = async () => { if (activeJobId) await invoke("cancel_job", { jobId: activeJobId }).catch((caught) => setError(String(caught))); };

  const saveResult = async () => {
    if (!media || !fullResult || !media.path) return;
    try {
      const stem = media.name.replace(/\.[^.]+$/, ""); const extension = media.kind === "image" ? "png" : "mp4";
      const suffix = media.kind === "image" ? "cutout" : `${screenColor}screen`;
      const destinationPath = await saveDialog({ defaultPath: media.path.replace(/[^\\/]+$/, `${stem}_${suffix}.${extension}`), filters: [{ name: media.kind === "image" ? "Transparent PNG" : "Screen video", extensions: [extension] }] });
      if (!destinationPath) return;
      setSavedPath(await invoke<string>("save_output", { sourcePath: fullResult.outputPath, destinationPath })); setError(null);
    } catch (caught) { setError(String(caught)); }
  };

  const applyCorrections = async () => {
    if (!fullResult || correctionStrokes.length === 0 || applyingCorrections) return;
    try {
      setApplyingCorrections(true); setError(null);
      const previous = fullResult;
      const outputPath = await invoke<string>("apply_image_corrections", { sourcePath: previous.outputPath, strokes: correctionStrokes });
      setFullResult({ ...previous, outputPath, durationMs: 0 });
      setPreviewMode("output"); setSavedPath(null); setCorrectionOpen(false); setCorrectionStrokes([]);
      discard(previous);
    } catch (caught) { setError(String(caught)); }
    finally { setApplyingCorrections(false); }
  };

  const applySeedCorrections = async () => {
    if (!seedResult || correctionStrokes.length === 0 || applyingCorrections) return;
    try {
      setApplyingCorrections(true); setError(null);
      const previousPath = seedResult.outputPath;
      const outputPath = await invoke<string>("apply_video_seed_corrections", { sourcePath: previousPath, strokes: correctionStrokes });
      setSeedResult({ ...seedResult, outputPath, durationMs: 0 });
      setCorrectionStrokes([]);
      if (isTauriRuntime()) void invoke("discard_output", { path: previousPath }).catch(() => undefined);
    } catch (caught) { setError(String(caught)); }
    finally { setApplyingCorrections(false); }
  };

  const displayResult = previewResult ?? fullResult;
  const previewSource = previewMode === "output" && displayResult ? convertFileSrc(displayResult.outputPath) : media?.url ?? "";
  const selectedTracker = bootstrap?.trackers.find((item) => item.tier === cutieTier);
  const processingDisabled = !bootstrap?.ready || status === "processing" || (media?.kind === "video" && trackingMode === "cutie" ? !selectedTracker?.installed || !seedResult : !!(requiredModel() && !requiredModel()?.installed));

  if (bootstrap && !bootstrap.ready) {
    const lite = bootstrap.models.find((item) => item.id === "generalLite");
    return <div className="app-shell onboarding"><div className="onboarding-card"><img className="onboarding-logo" src={rotoNowLogo} alt="Roto Now" /><p>WELCOME TO ROTO NOW</p><h1>One local model,<br /><span>then you are ready.</span></h1><p className="intro-copy">General Lite is required for general images and frame previews. It stays in your app-data folder and your media never leaves this computer.</p><div className="model-download-summary"><strong>{lite?.name ?? "General Lite"}</strong><span>{lite ? formatBytes(lite.size) : "Required model"}</span></div>{progress && <ProgressView progress={progress} />}<button className="process-button" disabled={!!activeJobId} onClick={() => downloadModel("generalLite")}><Download size={18} />{activeJobId ? "Downloading…" : "Download General Lite"}</button>{activeJobId && <button className="reset-button danger" onClick={cancelJob}>Cancel download</button>}{error && <p className="onboarding-error">Offline or download failed: {error}</p>}{error && !activeJobId && <button className="reset-button" onClick={() => downloadModel("generalLite")}>Retry</button>}</div></div>;
  }

  return <div className="app-shell">
    <header className="topbar"><div className="brand"><img className="brand-logo" src={rotoNowLogo} alt="" /><span>ROTO<span className="brand-accent">NOW</span></span></div><nav className="top-actions" aria-label="Application"><button className={`icon-button ${showModels ? "active" : ""}`} aria-label="Manage models" aria-expanded={showModels} title="Manage models" onClick={() => setShowModels((value) => !value)}><Settings2 size={18} /></button><button className={`icon-button ${helpOpen ? "active" : ""}`} aria-label="Help and about" aria-expanded={helpOpen} title="Help and about" onClick={() => setHelpOpen(true)}><CircleHelp size={18} /></button></nav></header>
    <main className="workspace">
      <input ref={inputRef} className="visually-hidden" type="file" accept="image/png,image/jpeg,image/webp,video/mp4,video/quicktime,video/webm" onChange={(event) => acceptFile(event.target.files?.[0])} />
      {showModels && <div className="modal-backdrop" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) setShowModels(false); }}><section className="model-manager panel" role="dialog" aria-modal="true" aria-labelledby="model-manager-title"><div className="model-manager-heading"><div><p>LOCAL MODELS</p><h2 id="model-manager-title">Model manager</h2><small>{bootstrap?.provider}</small></div><button className="icon-button" aria-label="Close model manager" autoFocus onClick={() => setShowModels(false)}><X size={17} /></button></div><div className="model-list">{bootstrap?.models.map((item) => <div className="model-row" key={item.id}><div><strong>{item.name}</strong><small>{formatBytes(item.size)} · {item.provider}</small></div><span className={`model-state ${item.installed ? "ready" : ""}`}>{item.state === "local" ? "Local" : item.installed ? "Ready" : item.state === "partial" ? "Partial" : "Optional"}</span>{item.installed && item.managed ? <><button onClick={() => downloadModel(item.id)} disabled={!!activeJobId}><RotateCcw size={14} /> Redownload</button><button className="remove-model" aria-label={`Remove ${item.name}`} onClick={() => removeModel(item.id)} disabled={!!activeJobId}><Trash2 size={14} /></button></> : !item.installed ? <button onClick={() => downloadModel(item.id)} disabled={!!activeJobId}><Download size={14} /> {item.state === "partial" ? "Resume" : "Download"}</button> : <span className="local-model-note">Development</span>}</div>)}{bootstrap?.trackers.map((item) => <div className="model-row" key={item.id}><div><strong>{item.name}</strong><small>{formatBytes(item.size)} · {item.width}×{item.height} internal · {item.provider} · sequential tracking</small></div><span className={`model-state ${item.installed ? "ready" : ""}`}>{item.state === "local" ? "Local" : item.installed ? "Ready" : item.state === "partial" ? "Partial" : "Optional"}</span>{item.installed && item.managed ? <><button onClick={() => downloadCutie(item.tier)} disabled={!!activeJobId}><RotateCcw size={14} /> Redownload</button><button className="remove-model" aria-label={`Remove ${item.name}`} onClick={() => removeCutie(item.tier)} disabled={!!activeJobId}><Trash2 size={14} /></button></> : !item.installed ? <button onClick={() => downloadCutie(item.tier)} disabled={!!activeJobId || !isTauriRuntime()}><Download size={14} /> {item.state === "partial" ? "Resume" : "Download"}</button> : <span className="local-model-note">Development</span>}</div>)}</div>{activeJobId && progress && <><ProgressView progress={progress} /><button className="reset-button danger" onClick={cancelJob}>Cancel</button></>}</section></div>}
      <section className="intro-row"><h1>Cut out the subject.<br /><span>Keep every detail.</span></h1></section>
      {media?.kind === "video" && seedEditorOpen && seedResult?.sourceFramePath ? <section className="editor-grid correction-grid"><div className="preview-panel panel"><div className="panel-heading"><div><span className="media-badge"><Brush size={14} /></span><h2>Cutie first-frame subject</h2><p>{media.name} · only the painted subject will be tracked</p></div><button className="icon-button" aria-label="Close first-frame mask editor" onClick={() => { setSeedEditorOpen(false); setCorrectionStrokes([]); }}><X size={18} /></button></div><div className="media-stage correction-stage checkerboard"><CorrectionCanvas inputSource={convertFileSrc(seedResult.sourceFramePath)} outputSource={convertFileSrc(seedResult.outputPath)} mode={correctionMode} radius={brushRadius} strokes={correctionStrokes} onChange={setCorrectionStrokes} /></div></div><aside className="controls-panel panel correction-controls"><div className="controls-title"><div><p>TRACKING SEED</p><h2>Mark the primary subject</h2></div><Brush size={19} /></div><label className="control-group"><span>Brush mode</span><div className="segmented correction-modes"><button className={correctionMode === "restore" ? "active" : ""} onClick={() => setCorrectionMode("restore")}><Brush size={14} /> Add subject</button><button className={correctionMode === "erase" ? "active" : ""} onClick={() => setCorrectionMode("erase")}><Eraser size={14} /> Remove</button></div><small>Keep only the person or object Cutie should follow. Remove the chair, bed, and background from this first frame.</small></label><label className="control-group range-group"><span><b>Brush size</b><output>{Math.round(brushRadius * 200)}%</output></span><input type="range" min="0.005" max="0.08" step="0.005" value={brushRadius} onChange={(event) => setBrushRadius(Number(event.target.value))} /></label><div className="correction-summary"><strong>{correctionStrokes.length}</strong><span>{correctionStrokes.length === 1 ? "brush stroke" : "brush strokes"}</span></div>{correctionStrokes.length > 0 && <button className="process-button" disabled={applyingCorrections} onClick={applySeedCorrections}>{applyingCorrections ? <><span className="mini-spinner" /> Applying…</> : <><Check size={18} /> Apply brushwork</>}</button>}<button className="preview-button" disabled={correctionStrokes.length > 0 || applyingCorrections} onClick={() => { setSeedEditorOpen(false); setCorrectionStrokes([]); }}><Check size={17} /> Use as tracking mask</button><button className="reset-button" disabled={correctionStrokes.length === 0} onClick={() => setCorrectionStrokes((current) => current.slice(0, -1))}><Undo2 size={14} /> Undo last stroke</button><button className="reset-button" disabled={correctionStrokes.length === 0} onClick={() => setCorrectionStrokes([])}><RotateCcw size={14} /> Clear brushwork</button><button className="reset-button danger" onClick={() => { clearSeed(); setStatus("ready"); }}><Trash2 size={14} /> Discard mask</button></aside></section> : media?.kind === "image" && correctionOpen && fullResult ? <section className="editor-grid correction-grid"><div className="preview-panel panel"><div className="panel-heading"><div><span className="media-badge"><Brush size={14} /></span><h2>Manual correction</h2><p>{media.name} · draw directly on the mask</p></div><button className="icon-button" aria-label="Close correction editor" onClick={() => { setCorrectionOpen(false); setCorrectionStrokes([]); }}><X size={18} /></button></div><div className="media-stage correction-stage checkerboard"><CorrectionCanvas inputSource={media.url} outputSource={convertFileSrc(fullResult.outputPath)} mode={correctionMode} radius={brushRadius} strokes={correctionStrokes} onChange={setCorrectionStrokes} /></div></div><aside className="controls-panel panel correction-controls"><div className="controls-title"><div><p>MASK EDITOR</p><h2>Refine the cutout</h2></div><Brush size={19} /></div><label className="control-group"><span>Brush mode</span><div className="segmented correction-modes"><button className={correctionMode === "restore" ? "active" : ""} onClick={() => setCorrectionMode("restore")}><Brush size={14} /> Restore</button><button className={correctionMode === "erase" ? "active" : ""} onClick={() => setCorrectionMode("erase")}><Eraser size={14} /> Erase</button></div><small>Restore brings original pixels back. Erase makes unwanted areas transparent.</small></label><label className="control-group range-group"><span><b>Brush size</b><output>{Math.round(brushRadius * 200)}%</output></span><input type="range" min="0.005" max="0.08" step="0.005" value={brushRadius} onChange={(event) => setBrushRadius(Number(event.target.value))} /></label><div className="correction-summary"><strong>{correctionStrokes.length}</strong><span>{correctionStrokes.length === 1 ? "brush stroke" : "brush strokes"}</span></div><button className="process-button" disabled={correctionStrokes.length === 0 || applyingCorrections} onClick={applyCorrections}>{applyingCorrections ? <><span className="mini-spinner" /> Applying…</> : <><Check size={18} /> Apply corrections</>}</button><button className="reset-button" disabled={correctionStrokes.length === 0} onClick={() => setCorrectionStrokes((current) => current.slice(0, -1))}><Undo2 size={14} /> Undo last stroke</button><button className="reset-button" disabled={correctionStrokes.length === 0} onClick={() => setCorrectionStrokes([])}><RotateCcw size={14} /> Clear brushwork</button><button className="reset-button" onClick={() => { setCorrectionOpen(false); setCorrectionStrokes([]); }}><X size={14} /> Cancel</button></aside></section> : !media ? <section className={`drop-zone ${dragging ? "is-dragging" : ""}`} onDragEnter={(event) => { event.preventDefault(); setDragging(true); }} onDragOver={(event) => event.preventDefault()} onDragLeave={(event) => { event.preventDefault(); setDragging(false); }} onDrop={(event) => { event.preventDefault(); setDragging(false); acceptFile(event.dataTransfer.files[0]); }}><div className="drop-glow" /><span className="upload-icon"><UploadCloud size={30} /></span><h2>Drop an image or video here</h2><p>or choose a file from your computer</p><button className="primary-button" onClick={browseFiles}><FolderOpen size={17} /> Browse files</button><div className="format-row"><span><FileImage size={14} /> PNG, JPG, WEBP</span><i /><span><Film size={14} /> MP4, MOV, WEBM</span></div></section> :
      <section className="editor-grid"><div className="preview-panel panel"><div className="panel-heading"><div><span className="media-badge">{media.kind === "image" ? <ImageIcon size={14} /> : <Film size={14} />}</span><h2>{media.name}</h2><p>{formatBytes(media.size)} · {status === "done" ? "Result ready" : "Ready to process"}</p></div><div className="preview-actions"><div className="preview-toggle" aria-label="Preview source"><button className={previewMode === "input" ? "active" : ""} aria-pressed={previewMode === "input"} onClick={() => setPreviewMode("input")}>Input</button><button className={previewMode === "output" ? "active" : ""} aria-pressed={previewMode === "output"} onClick={() => setPreviewMode("output")} disabled={!displayResult}>Output</button></div><button className="icon-button" aria-label="Close media" onClick={reset}><X size={18} /></button></div></div>
        <div className={`media-stage ${media.kind === "image" ? "checkerboard" : previewMode === "output" ? `screen-${screenColor}` : ""}`}>{media.kind === "image" || (media.kind === "video" && previewMode === "output" && displayResult?.preview) ? <img src={previewSource} alt={displayResult?.preview ? "Processed video preview frame" : previewMode === "output" ? "Background removed" : "Input"} /> : <VideoPlayer key={previewSource} source={previewSource} initialTime={playheadRef.current} onPlaybackTime={updatePlaybackRef} onTimeChange={updateSharedPlayhead} />}{status === "processing" && <div className="processing-overlay"><span className="spinner" /><strong>{progress?.message ?? "Preparing the cutout"}</strong>{progress && <ProgressView progress={progress} />}{activeJobId && <button className="cancel-overlay" onClick={cancelJob}><Pause size={14} /> Cancel</button>}</div>}{status === "done" && previewMode === "output" && <div className="done-badge"><Check size={15} /> {displayResult?.preview ? "Preview frame" : media.kind === "image" ? "Background removed" : "Full export"}</div>}</div></div>
        <aside className="controls-panel panel"><div className="controls-title"><div><p>OUTPUT SETTINGS</p><h2>Configure cutout</h2></div></div>
          {media.kind === "image" && <div className="control-group"><span>Detection model</span><div className="segmented">{(["General", "Anime"] as Model[]).map((item) => <button key={item} className={model === item ? "active" : ""} aria-pressed={model === item} onClick={() => { setModel(item); clearResults(); }}>{item}</button>)}</div><small>{model === "Anime" ? "Optimized for line art and stylized edges." : "Handles people, animals, products and objects."}</small></div>}
          {media.kind === "video" && <div className="control-group"><span>Video mask mode</span><div className="segmented"><button className={trackingMode === "temporal" ? "active" : ""} aria-pressed={trackingMode === "temporal"} onClick={() => { setTrackingMode("temporal"); clearResults(); }}>Temporal</button><button className={trackingMode === "cutie" ? "active" : ""} aria-pressed={trackingMode === "cutie"} onClick={() => { setTrackingMode("cutie"); clearResults(); }}>Primary Subject</button></div><small>{trackingMode === "cutie" ? "Cutie follows one marked subject through the sequence, reducing chair and background inclusions." : "Three-frame motion-gated stabilization keeps the existing per-frame segmentation workflow."}</small></div>}
          {media.kind === "video" && trackingMode === "temporal" && <div className="control-group"><span>Segmentation model</span><VideoModelSelect value={videoModel} ariaLabel="Select temporal segmentation model" onChange={(nextModel) => { setVideoModel(nextModel); clearResults(); }} /><small>{videoModel === "General Lite" ? "Fastest general-purpose model and included by default." : videoModel === "General Maximum" ? "Higher-detail general-purpose model with slower inference." : "Optimized for animation, line art, and stylized subjects."}</small>{requiredModel() && !requiredModel()?.installed && <small className="download-needed">{requiredModel()?.name} needs to be downloaded. <button onClick={() => setShowModels(true)}>Open models</button></small>}</div>}
          {media.kind === "video" && trackingMode === "cutie" && <div className="control-group"><span>Tracking quality</span><div className="segmented"><button className={cutieTier === "balanced" ? "active" : ""} aria-pressed={cutieTier === "balanced"} onClick={() => { setCutieTier("balanced"); clearResults(); }}>Balanced</button><button className={cutieTier === "high" ? "active" : ""} aria-pressed={cutieTier === "high"} onClick={() => { setCutieTier("high"); clearResults(); }}>High Detail</button></div><small>{cutieTier === "high" ? "960×544 internal tracking improves fine edges and thin structures, with slower processing and higher GPU memory use." : "640×368 internal tracking is the recommended balance of quality, speed, and memory."}</small></div>}
          {media.kind === "video" && trackingMode === "cutie" && <div className="seed-card"><div><strong>First-frame subject mask</strong><small>{seedResult ? `${seedResult.model} · ready for either tracker` : "Generate with a selected model or attach your own PNG"}</small></div><div className="seed-model-control"><span>Mask model</span><VideoModelSelect value={videoModel} ariaLabel="Select first-frame mask model" onChange={(nextModel) => { setVideoModel(nextModel); clearResults(); }} /></div>{requiredModel() && !requiredModel()?.installed && <small className="seed-download-needed">{requiredModel()?.name} is not installed. <button onClick={() => setShowModels(true)}>Open models</button></small>}{!selectedTracker?.installed && <button className="seed-action" onClick={() => setShowModels(true)}>Download {cutieTier === "high" ? "High Detail" : "Balanced"}</button>}<button className="seed-action" disabled={status === "processing"} onClick={() => startSeedJob()}><ImageIcon size={13} /> Generate mask</button><button className="seed-action" disabled={status === "processing"} onClick={attachSeedMask}><UploadCloud size={13} /> Attach PNG mask</button>{seedResult && <button className="seed-action ready" disabled={status === "processing"} onClick={() => { setCorrectionStrokes([]); setSeedEditorOpen(true); }}><Brush size={13} /> Refine current mask</button>}</div>}
          {media.kind === "video" && <div className="control-group"><span>Screen colour</span><div className="color-options"><button className={screenColor === "green" ? "selected" : ""} aria-pressed={screenColor === "green"} onClick={() => setScreenColor("green")}><i className="green-swatch" /><span>Green</span>{screenColor === "green" && <Check size={14} />}</button><button className={screenColor === "blue" ? "selected" : ""} aria-pressed={screenColor === "blue"} onClick={() => setScreenColor("blue")}><i className="blue-swatch" /><span>Blue</span>{screenColor === "blue" && <Check size={14} />}</button></div><small>{trackingMode === "cutie" ? "The confirmed first-frame mask is propagated across the full video; audio and timing are preserved." : "Motion-aware stabilization reduces mask flicker. Exports normalize rotation, frame timing, and audio sync."}</small></div>}
          {media.kind === "image" && <label className="control-group"><span>Quality</span><div className="quality-select"><Zap size={16} /><select value={quality} onChange={(event) => setQuality(event.target.value as Quality)}><option>Fast</option><option>Balanced</option><option>Maximum</option></select></div><small>{quality === "Fast" ? "General Lite · quickest processing." : quality === "Balanced" ? "General Lite · good detail with faster processing." : "General Maximum · highest detail with slower processing."}</small>{requiredModel() && !requiredModel()?.installed && <small className="download-needed">{requiredModel()?.name} needs to be downloaded. <button onClick={() => setShowModels(true)}>Open models</button></small>}</label>}
          {(media.kind === "image" || trackingMode === "temporal") && <label className="control-group range-group"><span><b>Edge detail</b><output>{edgeDetail}%</output></span><input type="range" min="0" max="100" value={edgeDetail} onChange={(event) => setEdgeDetail(Number(event.target.value))} /><small>Preserves fine hair, fur and soft edges.</small></label>}
          <div className="export-card"><span>{media.kind === "image" ? <FileImage size={20} /> : <Film size={20} />}</span><div><small>EXPORT FORMAT</small><strong>{media.kind === "image" ? "Transparent PNG" : `${screenColor === "green" ? "Green" : "Blue"} screen MP4`}</strong></div><Check size={16} /></div>
          {media.kind === "video" && <button className="preview-button" onClick={() => runJob(true)} disabled={status === "processing" || !!(requiredModel(true) && !requiredModel(true)?.installed)}><ImageIcon size={17} /> Preview frame at {formatTime(playhead)}</button>}
          {media.kind === "image" && fullResult && <button className="preview-button correction-button" onClick={() => { setCorrectionStrokes([]); setCorrectionOpen(true); }}><Brush size={17} /> Correct mask manually</button>}
          <button className="process-button" onClick={fullResult ? saveResult : () => runJob(false)} disabled={processingDisabled}>{fullResult ? <><Download size={18} />{savedPath ? "Save another copy" : `Save ${media.kind === "image" ? "PNG" : "full MP4"}`}</> : status === "processing" ? <><span className="mini-spinner" /> Processing…</> : <><WandSparkles size={18} /> {media.kind === "video" ? trackingMode === "cutie" ? "Track primary subject" : "Process full video" : "Remove background"}<ArrowRight size={17} /></>}</button>
          {displayResult && <p className="result-note" title={displayResult.pipeline}>{displayResult.preview ? "Frame preview" : "Full result"} · {displayResult.model} · processed in {(displayResult.durationMs / 1000).toFixed(1)}s · {displayResult.provider.replace("ExecutionProvider", "")} {displayResult.precision}{displayResult.width && displayResult.height ? ` · ${displayResult.width}×${displayResult.height}` : ""}{!displayResult.preview && displayResult.frameRate ? ` · ${displayResult.frameRate.toFixed(2)} fps` : ""}{!displayResult.preview && displayResult.mediaDurationSeconds != null ? ` · ${displayResult.mediaDurationSeconds.toFixed(2)}s media` : ""}{displayResult.frameCount ? ` · ${displayResult.frameCount} ${displayResult.frameCount === 1 ? "frame" : "frames"}` : ""}{media.kind === "video" && !displayResult.preview && displayResult.hasAudio != null ? displayResult.hasAudio ? " · audio" : " · silent" : ""}{displayResult.performance ? ` · inference ${(displayResult.performance.inferenceMs / Math.max(1, displayResult.frameCount ?? 1)).toFixed(0)}ms/frame` : ""}</p>}
          {fullResult && <button className="reset-button" onClick={() => runJob(false)}><WandSparkles size={14} /> Process again</button>}<button className="reset-button" onClick={reset}><RotateCcw size={14} /> Choose another file</button>
        </aside></section>}
      {helpOpen && <div className="modal-backdrop" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) setHelpOpen(false); }}><section className="help-dialog panel" role="dialog" aria-modal="true" aria-labelledby="help-title"><div className="help-heading"><div><p>ROTO NOW BETA</p><h2 id="help-title">Help &amp; about</h2></div><button className="icon-button" aria-label="Close help" autoFocus onClick={() => setHelpOpen(false)}><X size={18} /></button></div><div className="help-grid"><article><strong>1. Choose media</strong><span>Open a supported image or video. Your files stay on this computer.</span></article><article><strong>2. Configure</strong><span>Choose an image detection model, a Temporal segmentation model, or a first-frame mask model for Primary Subject tracking.</span></article><article><strong>3. Process and save</strong><span>Review Input and Output, refine image masks when needed, then choose where to save.</span></article></div><div className="privacy-note"><Check size={16} /><span><strong>Fully local processing</strong>No media is uploaded. Optional model downloads are the only network activity.</span></div><dl className="about-list"><div><dt>Version</dt><dd>{engineStatus ? `${engineStatus.version} beta` : "Development preview"}</dd></div><div><dt>Inference</dt><dd>{engineStatus?.inferenceEngine ?? "Native ONNX Runtime"}</dd></div><div><dt>Video</dt><dd>{engineStatus?.ffmpeg === "bundled" ? "Bundled FFmpeg" : engineStatus?.ffmpeg ?? "Bundled FFmpeg"}</dd></div></dl></section></div>}
      {error && <div className="error-toast" role="alert" aria-live="assertive"><X size={15} aria-hidden="true" /> <span>{error}</span><button aria-label="Dismiss error" onClick={() => setError(null)}><X size={13} /></button></div>}
    </main>
  </div>;
}

function VideoPlayer({ source, initialTime, onPlaybackTime, onTimeChange }: { source: string; initialTime: number; onPlaybackTime: (seconds: number) => void; onTimeChange: (seconds: number) => void; }) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const playerRef = useRef<HTMLDivElement>(null);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [fullscreen, setFullscreen] = useState(false);

  useEffect(() => {
    if (!playing) return;
    let animationFrame = 0;
    const updatePlayhead = () => {
      if (videoRef.current) {
        setCurrentTime(videoRef.current.currentTime);
        onPlaybackTime(videoRef.current.currentTime);
      }
      animationFrame = requestAnimationFrame(updatePlayhead);
    };
    animationFrame = requestAnimationFrame(updatePlayhead);
    return () => cancelAnimationFrame(animationFrame);
  }, [onPlaybackTime, playing]);

  useEffect(() => {
    const updateFullscreen = () => setFullscreen(document.fullscreenElement === playerRef.current);
    document.addEventListener("fullscreenchange", updateFullscreen);
    return () => document.removeEventListener("fullscreenchange", updateFullscreen);
  }, []);

  useEffect(() => {
    const video = videoRef.current;
    if (video) { video.setAttribute("src", source); video.load(); }
    return () => {
      if (!video) return;
      video.pause();
      video.removeAttribute("src");
      video.load();
    };
  }, [source]);

  const togglePlayback = () => {
    const video = videoRef.current;
    if (!video) return;
    if (video.paused) void video.play(); else video.pause();
  };

  const seek = (seconds: number) => {
    const video = videoRef.current;
    if (!video) return;
    video.currentTime = seconds;
    setCurrentTime(seconds);
    onTimeChange?.(seconds);
  };

  const syncInitialTime = (video: HTMLVideoElement) => {
    const videoDuration = Number.isFinite(video.duration) ? video.duration : 0;
    const target = Math.min(Math.max(initialTime, 0), videoDuration || initialTime);
    video.currentTime = target;
    setCurrentTime(target);
    onPlaybackTime(target);
    onTimeChange(target);
  };

  const toggleMute = () => {
    const video = videoRef.current;
    if (!video) return;
    video.muted = !video.muted;
    setMuted(video.muted);
  };

  const toggleFullscreen = () => {
    if (document.fullscreenElement) {
      void document.exitFullscreen();
    } else if (playerRef.current?.requestFullscreen) {
      void playerRef.current.requestFullscreen();
    }
  };

  return <div className="video-player" ref={playerRef}>
    <video ref={videoRef} src={source} preload="metadata" onClick={togglePlayback} onLoadedMetadata={(event) => { setDuration(Number.isFinite(event.currentTarget.duration) ? event.currentTarget.duration : 0); syncInitialTime(event.currentTarget); }} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setPlaying(false)} onTimeUpdate={(event) => { const seconds = event.currentTarget.currentTime; setCurrentTime(seconds); onPlaybackTime(seconds); onTimeChange(seconds); }} />
    <div className="video-controls">
      <button type="button" aria-label={playing ? "Pause video" : "Play video"} onClick={togglePlayback}>{playing ? <Pause size={15} /> : <Play size={15} />}</button>
      <span className="video-time">{formatTime(currentTime)}</span>
      <input className="video-seek" aria-label="Video position" type="range" min="0" max={Math.max(duration, 0.01)} step="0.01" value={Math.min(currentTime, Math.max(duration, 0.01))} onChange={(event) => seek(Number(event.target.value))} />
      <span className="video-time">{formatTime(duration)}</span>
      <button type="button" aria-label={muted ? "Unmute video" : "Mute video"} onClick={toggleMute}>{muted ? <VolumeX size={15} /> : <Volume2 size={15} />}</button>
      <button type="button" aria-label={fullscreen ? "Exit fullscreen" : "Enter fullscreen"} onClick={toggleFullscreen}>{fullscreen ? <Minimize2 size={14} /> : <Maximize2 size={14} />}</button>
    </div>
  </div>;
}

function CorrectionCanvas({ inputSource, outputSource, mode, radius, strokes, onChange }: { inputSource: string; outputSource: string; mode: CorrectionMode; radius: number; strokes: CorrectionStroke[]; onChange: (strokes: CorrectionStroke[]) => void; }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const activePointer = useRef<number | null>(null);
  const activeStroke = useRef<CorrectionStroke | null>(null);
  const baseStrokes = useRef<CorrectionStroke[]>([]);

  const backingRef = useRef<{ output: HTMLImageElement; original: HTMLCanvasElement } | null>(null);
  const [backingVersion, setBackingVersion] = useState(0);

  useEffect(() => {
    let disposed = false;
    const images: HTMLImageElement[] = [];
    const canvas = canvasRef.current;
    const load = (source: string) => new Promise<HTMLImageElement>((resolve, reject) => {
      const image = new Image();
      images.push(image);
      image.onload = () => resolve(image);
      image.onerror = () => reject(new Error("Could not load correction preview"));
      image.src = source;
    });
    void Promise.all([load(inputSource), load(outputSource)]).then(([input, output]) => {
      if (disposed || !canvas) return;
      canvas.width = output.naturalWidth;
      canvas.height = output.naturalHeight;
      const original = document.createElement("canvas");
      original.width = canvas.width;
      original.height = canvas.height;
      original.getContext("2d")?.drawImage(input, 0, 0, canvas.width, canvas.height);
      backingRef.current = { output, original };
      setBackingVersion((current) => current + 1);
    }).catch(() => undefined);
    return () => {
      disposed = true;
      for (const image of images) {
        image.onload = null;
        image.onerror = null;
        image.removeAttribute("src");
      }
      const backing = backingRef.current;
      if (backing) { backing.original.width = 0; backing.original.height = 0; }
      backingRef.current = null;
      if (canvas) { canvas.width = 0; canvas.height = 0; }
    };
  }, [inputSource, outputSource]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const backing = backingRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !backing || !context) return;
    context.clearRect(0, 0, canvas.width, canvas.height);
    context.drawImage(backing.output, 0, 0, canvas.width, canvas.height);
    const pattern = context.createPattern(backing.original, "no-repeat");
    if (!pattern) return;
    for (const stroke of strokes) paintCorrectionStroke(context, pattern, stroke, canvas.width, canvas.height);
  }, [backingVersion, strokes]);

  const pointFromEvent = (event: ReactPointerEvent<HTMLCanvasElement>): CorrectionPoint => {
    const bounds = event.currentTarget.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width)),
      y: Math.max(0, Math.min(1, (event.clientY - bounds.top) / bounds.height)),
    };
  };
  const startStroke = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    activePointer.current = event.pointerId;
    baseStrokes.current = strokes;
    activeStroke.current = { mode, radius, points: [pointFromEvent(event)] };
    onChange([...baseStrokes.current, activeStroke.current]);
  };
  const continueStroke = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (activePointer.current !== event.pointerId || !activeStroke.current) return;
    event.preventDefault();
    const previous = activeStroke.current.points.at(-1);
    const point = pointFromEvent(event);
    if (previous && Math.hypot(point.x - previous.x, point.y - previous.y) < 0.0015) return;
    activeStroke.current = { ...activeStroke.current, points: [...activeStroke.current.points, point] };
    onChange([...baseStrokes.current, activeStroke.current]);
  };
  const finishStroke = (event: ReactPointerEvent<HTMLCanvasElement>) => {
    if (activePointer.current !== event.pointerId) return;
    activePointer.current = null;
    activeStroke.current = null;
  };

  return <canvas ref={canvasRef} className={`correction-canvas mode-${mode}`} aria-label="Manual mask correction canvas" onPointerDown={startStroke} onPointerMove={continueStroke} onPointerUp={finishStroke} onPointerCancel={finishStroke} />;
}

function paintCorrectionStroke(context: CanvasRenderingContext2D, original: CanvasPattern, stroke: CorrectionStroke, width: number, height: number) {
  if (stroke.points.length === 0) return;
  const brush = stroke.radius * Math.min(width, height);
  context.save();
  context.globalCompositeOperation = stroke.mode === "erase" ? "destination-out" : "source-over";
  context.strokeStyle = stroke.mode === "erase" ? "#000" : original;
  context.fillStyle = stroke.mode === "erase" ? "#000" : original;
  context.lineCap = "round";
  context.lineJoin = "round";
  context.lineWidth = brush * 2;
  if (stroke.points.length === 1) {
    const point = stroke.points[0];
    context.beginPath();
    context.arc(point.x * width, point.y * height, brush, 0, Math.PI * 2);
    context.fill();
  } else {
    context.beginPath();
    stroke.points.forEach((point, index) => index === 0 ? context.moveTo(point.x * width, point.y * height) : context.lineTo(point.x * width, point.y * height));
    context.stroke();
  }
  context.restore();
}

function ProgressView({ progress }: { progress: JobProgress }) {
  return <div className="job-progress" role="progressbar" aria-label={progress.message} aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress.percent == null ? undefined : Math.round(progress.percent)}><div className={`progress-track ${progress.percent == null ? "indeterminate" : ""}`}><i style={progress.percent == null ? undefined : { width: `${progress.percent}%` }} /></div><small>{progress.percent == null ? progress.message : `${Math.round(progress.percent)}%`}{progress.etaSeconds != null ? ` · about ${progress.etaSeconds}s left` : ""}</small></div>;
}

export default App;
