import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ArrowRight, Brush, Check, CircleHelp, Download, Eraser, FileImage, Film, FolderOpen, Image as ImageIcon, Layers3, Maximize2, Minimize2, Pause, Play, RotateCcw, Settings2, Trash2, Undo2, UploadCloud, Volume2, VolumeX, WandSparkles, X, Zap } from "lucide-react";

type MediaKind = "image" | "video";
type Quality = "Fast" | "Balanced" | "Maximum";
type Model = "General" | "Anime";
type ModelId = "generalLite" | "general" | "anime";
type ScreenColor = "green" | "blue";
type PreviewMode = "input" | "output";
type CorrectionMode = "restore" | "erase";

interface ImportedMedia { file?: File; path?: string; name: string; size: number; kind: MediaKind; url: string; }
interface NativeMediaInfo { path: string; name: string; size: number; kind: MediaKind; previewDataUrl?: string; }
interface ProcessResult { outputPath: string; model: string; provider: string; durationMs: number; frameCount?: number; width?: number; height?: number; frameRate?: number; mediaDurationSeconds?: number; hasAudio?: boolean; preview: boolean; }
interface ModelStatus { id: ModelId; name: string; size: number; installed: boolean; managed: boolean; state: string; provider: string; }
interface BootstrapStatus { ready: boolean; provider: string; models: ModelStatus[]; }
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

function App() {
  const inputRef = useRef<HTMLInputElement>(null);
  const playheadRef = useRef(0);
  const [playhead, setPlayhead] = useState(0);
  const [media, setMedia] = useState<ImportedMedia | null>(null);
  const [dragging, setDragging] = useState(false);
  const [model, setModel] = useState<Model>("General");
  const [quality, setQuality] = useState<Quality>("Balanced");
  const [screenColor, setScreenColor] = useState<ScreenColor>("green");
  const [edgeDetail, setEdgeDetail] = useState(72);
  const [status, setStatus] = useState<"idle" | "ready" | "processing" | "done">("idle");
  const [error, setError] = useState<string | null>(null);
  const [fullResult, setFullResult] = useState<ProcessResult | null>(null);
  const [previewResult, setPreviewResult] = useState<ProcessResult | null>(null);
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

  const refreshBootstrap = async () => {
    if (!isTauriRuntime()) { setBootstrap({ ready: true, provider: "Browser mock", models: [] }); return; }
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
      if (payload.result.preview) setPreviewResult(payload.result); else { setFullResult(payload.result); setCorrectionOpen(false); setCorrectionStrokes([]); }
      setPreviewMode("output");
      setStatus("done");
    }).then((cleanup) => { unlisten = cleanup; });
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

  const acceptFile = (file?: File) => {
    if (!file) return;
    if (!isSupported(file)) { setError("Choose a supported image or video file."); return; }
    if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
    clearResults();
    const kind: MediaKind = file.type.startsWith("video/") ? "video" : "image";
    setMedia({ file, name: file.name, size: file.size, kind, url: URL.createObjectURL(file) });
    setError(null); setStatus("ready"); playheadRef.current = 0; setPlayhead(0);
  };

  const browseFiles = async () => {
    if (!isTauriRuntime()) { inputRef.current?.click(); return; }
    try {
      const selected = await openDialog({ multiple: false, directory: false, filters: [{ name: "Images and videos", extensions: ["png", "jpg", "jpeg", "webp", "mp4", "mov", "webm"] }] });
      if (!selected || Array.isArray(selected)) return;
      const info = await invoke<NativeMediaInfo>("inspect_media", { path: selected });
      if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
      clearResults();
      setMedia({ path: info.path, name: info.name, size: info.size, kind: info.kind, url: info.previewDataUrl ?? convertFileSrc(info.path) });
      setError(null); setStatus("ready"); playheadRef.current = 0; setPlayhead(0);
    } catch (caught) { setError(String(caught)); }
  };

  const reset = () => {
    if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
    clearResults(); setMedia(null); setStatus("idle"); setError(null); playheadRef.current = 0; setPlayhead(0);
    if (inputRef.current) inputRef.current.value = "";
  };

  const requiredModelId = (forPreview = false): ModelId => model === "Anime" ? "anime" : (!forPreview && quality === "Maximum" ? "general" : "generalLite");
  const requiredModel = (forPreview = false) => bootstrap?.models.find((item) => item.id === requiredModelId(forPreview));

  const downloadModel = async (modelId: ModelId) => {
    try { setError(null); setProgress({ phase: "downloading", message: "Starting model download" }); setActiveJobId(await invoke<string>("download_model", { modelId })); }
    catch (caught) { setProgress(null); setError(String(caught)); }
  };

  const removeModel = async (modelId: ModelId) => {
    try { await invoke("remove_model", { modelId }); await refreshBootstrap(); }
    catch (caught) { setError(String(caught)); }
  };

  const runJob = async (fastPreview = false) => {
    if (!media || status === "processing") return;
    const needed = requiredModel(fastPreview);
    if (needed && !needed.installed) { setError(`${needed.name} must be downloaded first.`); setShowModels(true); return; }
    if (!isTauriRuntime() || !media.path) { setStatus("processing"); window.setTimeout(() => setStatus("done"), 900); return; }
    try {
      setError(null); setSavedPath(null); setProgress({ phase: "starting", message: fastPreview ? "Preparing preview frame" : "Preparing job" }); setStatus("processing");
      if (fastPreview) { discard(previewResult); setPreviewResult(null); }
      else { clearResults(); }
      const command = media.kind === "image" ? "start_image_job" : "start_video_job";
      const id = await invoke<string>(command, {
        inputPath: media.path, model, quality, edgeDetail,
        ...(media.kind === "video" ? { screenColor, preview: fastPreview, startSeconds: fastPreview ? playheadRef.current : 0 } : {}),
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

  const displayResult = previewResult ?? fullResult;
  const previewSource = previewMode === "output" && displayResult ? convertFileSrc(displayResult.outputPath) : media?.url ?? "";
  const processingDisabled = !bootstrap?.ready || status === "processing" || !!(requiredModel() && !requiredModel()?.installed);

  if (bootstrap && !bootstrap.ready) {
    const lite = bootstrap.models.find((item) => item.id === "generalLite");
    return <div className="app-shell onboarding"><div className="onboarding-card"><span className="brand-mark"><Layers3 size={24} /></span><p>WELCOME TO ROTO NOW</p><h1>One local model,<br /><span>then you are ready.</span></h1><p className="intro-copy">General Lite is required for general images and frame previews. It stays in your app-data folder and your media never leaves this computer.</p><div className="model-download-summary"><strong>{lite?.name ?? "General Lite"}</strong><span>{lite ? formatBytes(lite.size) : "Required model"}</span></div>{progress && <ProgressView progress={progress} />}<button className="process-button" disabled={!!activeJobId} onClick={() => downloadModel("generalLite")}><Download size={18} />{activeJobId ? "Downloading…" : "Download General Lite"}</button>{activeJobId && <button className="reset-button danger" onClick={cancelJob}>Cancel download</button>}{error && <p className="onboarding-error">Offline or download failed: {error}</p>}{error && !activeJobId && <button className="reset-button" onClick={() => downloadModel("generalLite")}>Retry</button>}</div></div>;
  }

  return <div className="app-shell">
    <header className="topbar"><div className="brand"><span className="brand-mark"><Layers3 size={18} /></span><span>ROTO<span className="brand-accent">NOW</span></span><span className="beta-pill">BETA</span></div><nav className="top-actions" aria-label="Application"><button className={`icon-button ${showModels ? "active" : ""}`} aria-label="Manage models" aria-expanded={showModels} title="Manage models" onClick={() => setShowModels((value) => !value)}><Settings2 size={18} /></button><button className={`icon-button ${helpOpen ? "active" : ""}`} aria-label="Help and about" aria-expanded={helpOpen} title="Help and about" onClick={() => setHelpOpen(true)}><CircleHelp size={18} /></button></nav></header>
    <main className="workspace">
      <input ref={inputRef} className="visually-hidden" type="file" accept="image/png,image/jpeg,image/webp,video/mp4,video/quicktime,video/webm" onChange={(event) => acceptFile(event.target.files?.[0])} />
      {showModels && <section className="model-manager panel" aria-labelledby="model-manager-title"><div className="model-manager-heading"><div><p>LOCAL MODELS</p><h2 id="model-manager-title">Model manager</h2><small>{bootstrap?.provider}</small></div><button className="icon-button" aria-label="Close model manager" onClick={() => setShowModels(false)}><X size={17} /></button></div><div className="model-list">{bootstrap?.models.map((item) => <div className="model-row" key={item.id}><div><strong>{item.name}</strong><small>{formatBytes(item.size)} · {item.provider} · {item.state === "local" ? "Local test model" : item.installed ? "Ready" : item.state === "partial" ? "Resume available" : "Not installed"}</small></div><span className={`model-state ${item.installed ? "ready" : ""}`}>{item.state === "local" ? "Local" : item.installed ? "Ready" : item.state === "partial" ? "Partial" : "Optional"}</span>{item.installed && item.managed ? <><button onClick={() => downloadModel(item.id)} disabled={!!activeJobId}><RotateCcw size={14} /> Redownload</button><button className="remove-model" aria-label={`Remove ${item.name}`} onClick={() => removeModel(item.id)} disabled={!!activeJobId}><Trash2 size={14} /></button></> : !item.installed ? <button onClick={() => downloadModel(item.id)} disabled={!!activeJobId}><Download size={14} /> {item.state === "partial" ? "Resume" : "Download"}</button> : <span className="local-model-note">Development</span>}</div>)}</div>{activeJobId && progress && <><ProgressView progress={progress} /><button className="reset-button danger" onClick={cancelJob}>Cancel</button></>}</section>}
      <section className="intro-row"><h1>Cut out the subject.<br /><span>Keep every detail.</span></h1><p className="intro-copy">Turn images into transparent PNGs and videos into clean green or blue screen footage—entirely on your device.</p></section>
      {media?.kind === "image" && correctionOpen && fullResult ? <section className="editor-grid correction-grid"><div className="preview-panel panel"><div className="panel-heading"><div><span className="media-badge"><Brush size={14} /></span><h2>Manual correction</h2><p>{media.name} · draw directly on the mask</p></div><button className="icon-button" aria-label="Close correction editor" onClick={() => { setCorrectionOpen(false); setCorrectionStrokes([]); }}><X size={18} /></button></div><div className="media-stage correction-stage checkerboard"><CorrectionCanvas inputSource={media.url} outputSource={convertFileSrc(fullResult.outputPath)} mode={correctionMode} radius={brushRadius} strokes={correctionStrokes} onChange={setCorrectionStrokes} /></div></div><aside className="controls-panel panel correction-controls"><div className="controls-title"><div><p>MASK EDITOR</p><h2>Refine the cutout</h2></div><Brush size={19} /></div><label className="control-group"><span>Brush mode</span><div className="segmented correction-modes"><button className={correctionMode === "restore" ? "active" : ""} onClick={() => setCorrectionMode("restore")}><Brush size={14} /> Restore</button><button className={correctionMode === "erase" ? "active" : ""} onClick={() => setCorrectionMode("erase")}><Eraser size={14} /> Erase</button></div><small>Restore brings original pixels back. Erase makes unwanted areas transparent.</small></label><label className="control-group range-group"><span><b>Brush size</b><output>{Math.round(brushRadius * 200)}%</output></span><input type="range" min="0.005" max="0.08" step="0.005" value={brushRadius} onChange={(event) => setBrushRadius(Number(event.target.value))} /></label><div className="correction-summary"><strong>{correctionStrokes.length}</strong><span>{correctionStrokes.length === 1 ? "brush stroke" : "brush strokes"}</span></div><button className="process-button" disabled={correctionStrokes.length === 0 || applyingCorrections} onClick={applyCorrections}>{applyingCorrections ? <><span className="mini-spinner" /> Applying…</> : <><Check size={18} /> Apply corrections</>}</button><button className="reset-button" disabled={correctionStrokes.length === 0} onClick={() => setCorrectionStrokes((current) => current.slice(0, -1))}><Undo2 size={14} /> Undo last stroke</button><button className="reset-button" disabled={correctionStrokes.length === 0} onClick={() => setCorrectionStrokes([])}><RotateCcw size={14} /> Clear brushwork</button><button className="reset-button" onClick={() => { setCorrectionOpen(false); setCorrectionStrokes([]); }}><X size={14} /> Cancel</button></aside></section> : !media ? <section className={`drop-zone ${dragging ? "is-dragging" : ""}`} onDragEnter={(event) => { event.preventDefault(); setDragging(true); }} onDragOver={(event) => event.preventDefault()} onDragLeave={(event) => { event.preventDefault(); setDragging(false); }} onDrop={(event) => { event.preventDefault(); setDragging(false); acceptFile(event.dataTransfer.files[0]); }}><div className="drop-glow" /><span className="upload-icon"><UploadCloud size={30} /></span><h2>Drop an image or video here</h2><p>or choose a file from your computer</p><button className="primary-button" onClick={browseFiles}><FolderOpen size={17} /> Browse files</button><div className="format-row"><span><FileImage size={14} /> PNG, JPG, WEBP</span><i /><span><Film size={14} /> MP4, MOV, WEBM</span></div></section> :
      <section className="editor-grid"><div className="preview-panel panel"><div className="panel-heading"><div><span className="media-badge">{media.kind === "image" ? <ImageIcon size={14} /> : <Film size={14} />}</span><h2>{media.name}</h2><p>{formatBytes(media.size)} · {status === "done" ? "Result ready" : "Ready to process"}</p></div><div className="preview-actions"><div className="preview-toggle" aria-label="Preview source"><button className={previewMode === "input" ? "active" : ""} aria-pressed={previewMode === "input"} onClick={() => setPreviewMode("input")}>Input</button><button className={previewMode === "output" ? "active" : ""} aria-pressed={previewMode === "output"} onClick={() => setPreviewMode("output")} disabled={!displayResult}>Output</button></div><button className="icon-button" aria-label="Close media" onClick={reset}><X size={18} /></button></div></div>
        <div className={`media-stage ${media.kind === "image" ? "checkerboard" : previewMode === "output" ? `screen-${screenColor}` : ""}`}>{media.kind === "image" || (media.kind === "video" && previewMode === "output" && displayResult?.preview) ? <img src={previewSource} alt={displayResult?.preview ? "Processed video preview frame" : previewMode === "output" ? "Background removed" : "Input"} /> : <VideoPlayer key={previewSource} source={previewSource} onTimeChange={previewMode === "input" ? (seconds) => { playheadRef.current = seconds; setPlayhead(seconds); } : undefined} />}{status === "processing" && <div className="processing-overlay"><span className="spinner" /><strong>{progress?.message ?? "Preparing the cutout"}</strong>{progress && <ProgressView progress={progress} />}{activeJobId && <button className="cancel-overlay" onClick={cancelJob}><Pause size={14} /> Cancel</button>}</div>}{status === "done" && previewMode === "output" && <div className="done-badge"><Check size={15} /> {displayResult?.preview ? "Preview frame" : media.kind === "image" ? "Background removed" : "Full export"}</div>}</div></div>
        <aside className="controls-panel panel"><div className="controls-title"><div><p>OUTPUT SETTINGS</p><h2>Configure cutout</h2></div><Settings2 size={19} /></div>
          <div className="control-group"><span>Detection model</span><div className="segmented">{(["General", "Anime"] as Model[]).map((item) => <button key={item} className={model === item ? "active" : ""} aria-pressed={model === item} onClick={() => setModel(item)}>{item}</button>)}</div><small>{model === "Anime" ? "Optimized for line art and stylized edges." : "Handles people, animals, products and objects."}</small></div>
          {media.kind === "video" && <div className="control-group"><span>Screen colour</span><div className="color-options"><button className={screenColor === "green" ? "selected" : ""} aria-pressed={screenColor === "green"} onClick={() => setScreenColor("green")}><i className="green-swatch" /><span>Green</span>{screenColor === "green" && <Check size={14} />}</button><button className={screenColor === "blue" ? "selected" : ""} aria-pressed={screenColor === "blue"} onClick={() => setScreenColor("blue")}><i className="blue-swatch" /><span>Blue</span>{screenColor === "blue" && <Check size={14} />}</button></div><small>Motion-aware stabilization reduces mask flicker. Exports normalize rotation, frame timing, and audio sync.</small></div>}
          <label className="control-group"><span>Quality</span><div className="quality-select"><Zap size={16} /><select value={quality} onChange={(event) => setQuality(event.target.value as Quality)}><option>Fast</option><option>Balanced</option><option>Maximum</option></select></div><small>{quality === "Fast" ? `General Lite · quicker mask resampling${media.kind === "video" ? " · faster encoding" : ""}.` : quality === "Balanced" ? `General Lite · detailed mask resampling${media.kind === "video" ? " · balanced encoding" : ""}.` : `General Maximum · highest detail${media.kind === "video" ? " · slower high-quality encoding" : ""}.`}</small>{requiredModel() && !requiredModel()?.installed && <small className="download-needed">{requiredModel()?.name} needs to be downloaded. <button onClick={() => setShowModels(true)}>Open models</button></small>}</label>
          <label className="control-group range-group"><span><b>Edge detail</b><output>{edgeDetail}%</output></span><input type="range" min="0" max="100" value={edgeDetail} onChange={(event) => setEdgeDetail(Number(event.target.value))} /><small>Preserves fine hair, fur and soft edges.</small></label>
          <div className="export-card"><span>{media.kind === "image" ? <FileImage size={20} /> : <Film size={20} />}</span><div><small>EXPORT FORMAT</small><strong>{media.kind === "image" ? "Transparent PNG" : `${screenColor === "green" ? "Green" : "Blue"} screen MP4`}</strong></div><Check size={16} /></div>
          {media.kind === "video" && <button className="preview-button" onClick={() => runJob(true)} disabled={status === "processing" || !!(requiredModel(true) && !requiredModel(true)?.installed)}><ImageIcon size={17} /> Preview frame at {formatTime(playhead)}</button>}
          {media.kind === "image" && fullResult && <button className="preview-button correction-button" onClick={() => { setCorrectionStrokes([]); setCorrectionOpen(true); }}><Brush size={17} /> Correct mask manually</button>}
          <button className="process-button" onClick={fullResult ? saveResult : () => runJob(false)} disabled={processingDisabled}>{fullResult ? <><Download size={18} />{savedPath ? "Save another copy" : `Save ${media.kind === "image" ? "PNG" : "full MP4"}`}</> : status === "processing" ? <><span className="mini-spinner" /> Processing…</> : <><WandSparkles size={18} /> {media.kind === "video" ? "Process full video" : "Remove background"}<ArrowRight size={17} /></>}</button>
          {displayResult && <p className="result-note">{displayResult.preview ? "Frame preview" : "Full result"} · {displayResult.model} · processed in {(displayResult.durationMs / 1000).toFixed(1)}s · {displayResult.provider.replace("ExecutionProvider", "")}{displayResult.width && displayResult.height ? ` · ${displayResult.width}×${displayResult.height}` : ""}{!displayResult.preview && displayResult.frameRate ? ` · ${displayResult.frameRate.toFixed(2)} fps` : ""}{!displayResult.preview && displayResult.mediaDurationSeconds != null ? ` · ${displayResult.mediaDurationSeconds.toFixed(2)}s media` : ""}{displayResult.frameCount ? ` · ${displayResult.frameCount} ${displayResult.frameCount === 1 ? "frame" : "frames"}` : ""}{media.kind === "video" && !displayResult.preview && displayResult.hasAudio != null ? displayResult.hasAudio ? " · audio" : " · silent" : ""}</p>}
          {fullResult && <button className="reset-button" onClick={() => runJob(false)}><WandSparkles size={14} /> Process again</button>}<button className="reset-button" onClick={reset}><RotateCcw size={14} /> Choose another file</button>
        </aside></section>}
      {helpOpen && <div className="modal-backdrop" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) setHelpOpen(false); }}><section className="help-dialog panel" role="dialog" aria-modal="true" aria-labelledby="help-title"><div className="help-heading"><div><p>ROTO NOW BETA</p><h2 id="help-title">Help &amp; about</h2></div><button className="icon-button" aria-label="Close help" autoFocus onClick={() => setHelpOpen(false)}><X size={18} /></button></div><div className="help-grid"><article><strong>1. Choose media</strong><span>Open a supported image or video. Your files stay on this computer.</span></article><article><strong>2. Configure</strong><span>Choose General for photos and objects, or Anime for stylized artwork. Video previews process the frame at the playhead.</span></article><article><strong>3. Process and save</strong><span>Review Input and Output, refine image masks when needed, then choose where to save.</span></article></div><div className="privacy-note"><Check size={16} /><span><strong>Fully local processing</strong>No media is uploaded. Optional model downloads are the only network activity.</span></div><dl className="about-list"><div><dt>Version</dt><dd>{engineStatus ? `${engineStatus.version} beta` : "Development preview"}</dd></div><div><dt>Inference</dt><dd>{engineStatus?.inferenceEngine ?? "Native ONNX Runtime"}</dd></div><div><dt>Video</dt><dd>{engineStatus?.ffmpeg === "bundled" ? "Bundled FFmpeg" : engineStatus?.ffmpeg ?? "Bundled FFmpeg"}</dd></div></dl></section></div>}
      {error && <div className="error-toast" role="alert" aria-live="assertive"><X size={15} aria-hidden="true" /> <span>{error}</span><button aria-label="Dismiss error" onClick={() => setError(null)}><X size={13} /></button></div>}
    </main>
  </div>;
}

function VideoPlayer({ source, onTimeChange }: { source: string; onTimeChange?: (seconds: number) => void; }) {
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
      if (videoRef.current) setCurrentTime(videoRef.current.currentTime);
      animationFrame = requestAnimationFrame(updatePlayhead);
    };
    animationFrame = requestAnimationFrame(updatePlayhead);
    return () => cancelAnimationFrame(animationFrame);
  }, [playing]);

  useEffect(() => {
    const updateFullscreen = () => setFullscreen(document.fullscreenElement === playerRef.current);
    document.addEventListener("fullscreenchange", updateFullscreen);
    return () => document.removeEventListener("fullscreenchange", updateFullscreen);
  }, []);

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
    <video ref={videoRef} src={source} preload="metadata" onClick={togglePlayback} onLoadedMetadata={(event) => setDuration(Number.isFinite(event.currentTarget.duration) ? event.currentTarget.duration : 0)} onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setPlaying(false)} onTimeUpdate={(event) => { const seconds = event.currentTarget.currentTime; setCurrentTime(seconds); onTimeChange?.(seconds); }} />
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

  useEffect(() => {
    let disposed = false;
    const load = (source: string) => new Promise<HTMLImageElement>((resolve, reject) => {
      const image = new Image();
      image.onload = () => resolve(image);
      image.onerror = () => reject(new Error("Could not load correction preview"));
      image.src = source;
    });
    void Promise.all([load(inputSource), load(outputSource)]).then(([input, output]) => {
      if (disposed || !canvasRef.current) return;
      const canvas = canvasRef.current;
      canvas.width = output.naturalWidth;
      canvas.height = output.naturalHeight;
      const context = canvas.getContext("2d");
      if (!context) return;
      context.clearRect(0, 0, canvas.width, canvas.height);
      context.drawImage(output, 0, 0, canvas.width, canvas.height);
      const original = document.createElement("canvas");
      original.width = canvas.width;
      original.height = canvas.height;
      original.getContext("2d")?.drawImage(input, 0, 0, canvas.width, canvas.height);
      const pattern = context.createPattern(original, "no-repeat");
      if (!pattern) return;
      for (const stroke of strokes) paintCorrectionStroke(context, pattern, stroke, canvas.width, canvas.height);
    }).catch(() => undefined);
    return () => { disposed = true; };
  }, [inputSource, outputSource, strokes]);

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
