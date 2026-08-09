import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ArrowRight, Check, CircleHelp, Download, FileImage, Film, FolderOpen, Image as ImageIcon, Layers3, Moon, Pause, RotateCcw, Settings2, Trash2, UploadCloud, WandSparkles, X, Zap } from "lucide-react";

type MediaKind = "image" | "video";
type Quality = "Fast" | "Balanced" | "Maximum";
type Model = "Auto" | "General" | "Anime";
type ModelId = "generalLite" | "general" | "anime";
type ScreenColor = "green" | "blue";
type PreviewMode = "input" | "output";

interface ImportedMedia { file?: File; path?: string; name: string; size: number; kind: MediaKind; url: string; }
interface NativeMediaInfo { path: string; name: string; size: number; kind: MediaKind; previewDataUrl?: string; }
interface ProcessResult { outputPath: string; model: string; provider: string; durationMs: number; frameCount?: number; preview: boolean; }
interface ModelStatus { id: ModelId; name: string; size: number; installed: boolean; state: string; provider: string; }
interface BootstrapStatus { ready: boolean; provider: string; models: ModelStatus[]; }
interface JobProgress { phase: string; completed?: number; total?: number; percent?: number; etaSeconds?: number; message: string; }
type JobEvent =
  | ({ jobId: string; type: "progress" } & JobProgress)
  | { jobId: string; type: "completed"; result: ProcessResult }
  | { jobId: string; type: "failed"; error: string }
  | { jobId: string; type: "cancelled" };

const isTauriRuntime = () => "__TAURI_INTERNALS__" in window;
const formatBytes = (bytes: number) => bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / (1024 * 1024)).toFixed(bytes > 1024 ** 3 ? 2 : 0)} MB`;
const isSupported = (file: File) => file.type.startsWith("image/") || file.type.startsWith("video/");

function App() {
  const inputRef = useRef<HTMLInputElement>(null);
  const playheadRef = useRef(0);
  const [playhead, setPlayhead] = useState(0);
  const [media, setMedia] = useState<ImportedMedia | null>(null);
  const [dragging, setDragging] = useState(false);
  const [model, setModel] = useState<Model>("Auto");
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

  const refreshBootstrap = async () => {
    if (!isTauriRuntime()) { setBootstrap({ ready: true, provider: "Browser mock", models: [] }); return; }
    setBootstrap(await invoke<BootstrapStatus>("get_bootstrap_status"));
  };

  useEffect(() => {
    void refreshBootstrap().catch((caught) => setError(String(caught)));
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
      if (payload.result.preview) setPreviewResult(payload.result); else setFullResult(payload.result);
      setPreviewMode("output");
      setStatus("done");
    }).then((cleanup) => { unlisten = cleanup; });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  const discard = (result: ProcessResult | null) => {
    if (result?.outputPath && isTauriRuntime()) void invoke("discard_output", { path: result.outputPath }).catch(() => undefined);
  };
  const clearResults = () => { discard(fullResult); discard(previewResult); setFullResult(null); setPreviewResult(null); setPreviewMode("input"); setSavedPath(null); };

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
      setError(null); setSavedPath(null); setProgress({ phase: "starting", message: fastPreview ? "Preparing one-second preview" : "Preparing job" }); setStatus("processing");
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

  const displayResult = previewResult ?? fullResult;
  const previewSource = previewMode === "output" && displayResult ? convertFileSrc(displayResult.outputPath) : media?.url ?? "";
  const processingDisabled = !bootstrap?.ready || status === "processing" || !!(requiredModel() && !requiredModel()?.installed);

  if (bootstrap && !bootstrap.ready) {
    const lite = bootstrap.models.find((item) => item.id === "generalLite");
    return <div className="app-shell onboarding"><div className="onboarding-card"><span className="brand-mark"><Layers3 size={24} /></span><p>WELCOME TO ROTO NOW</p><h1>One local model,<br /><span>then you are ready.</span></h1><p className="intro-copy">General Lite is required for images and fast previews. It stays in your app-data folder and your media never leaves this computer.</p><div className="model-download-summary"><strong>{lite?.name ?? "General Lite"}</strong><span>{lite ? formatBytes(lite.size) : "Required model"}</span></div>{progress && <ProgressView progress={progress} />}<button className="process-button" disabled={!!activeJobId} onClick={() => downloadModel("generalLite")}><Download size={18} />{activeJobId ? "Downloading…" : "Download General Lite"}</button>{activeJobId && <button className="reset-button danger" onClick={cancelJob}>Cancel download</button>}{error && <p className="onboarding-error">Offline or download failed: {error}</p>}{error && !activeJobId && <button className="reset-button" onClick={() => downloadModel("generalLite")}>Retry</button>}</div></div>;
  }

  return <div className="app-shell">
    <header className="topbar"><div className="brand"><span className="brand-mark"><Layers3 size={18} /></span><span>ROTO<span className="brand-accent">NOW</span></span><span className="prototype-pill">PUBLIC ALPHA</span></div><nav className="top-actions"><button className="icon-button" aria-label="Manage models" onClick={() => setShowModels((value) => !value)}><Settings2 size={18} /></button><button className="icon-button" aria-label="Help"><CircleHelp size={18} /></button><button className="icon-button" aria-label="Toggle theme"><Moon size={18} /></button></nav></header>
    <main className="workspace">
      <input ref={inputRef} className="visually-hidden" type="file" accept="image/png,image/jpeg,image/webp,video/mp4,video/quicktime,video/webm" onChange={(event) => acceptFile(event.target.files?.[0])} />
      {showModels && <section className="model-manager panel"><div className="model-manager-heading"><div><p>LOCAL MODELS</p><h2>Model manager</h2><small>{bootstrap?.provider}</small></div><button className="icon-button" onClick={() => setShowModels(false)}><X size={17} /></button></div><div className="model-list">{bootstrap?.models.map((item) => <div className="model-row" key={item.id}><div><strong>{item.name}</strong><small>{formatBytes(item.size)} · {item.provider} · {item.installed ? "Ready" : item.state === "partial" ? "Resume available" : "Not installed"}</small></div><span className={`model-state ${item.installed ? "ready" : ""}`}>{item.installed ? "Ready" : item.state === "partial" ? "Partial" : "Optional"}</span>{item.installed ? <><button onClick={() => downloadModel(item.id)} disabled={!!activeJobId}><RotateCcw size={14} /> Redownload</button><button className="remove-model" onClick={() => removeModel(item.id)} disabled={!!activeJobId}><Trash2 size={14} /></button></> : <button onClick={() => downloadModel(item.id)} disabled={!!activeJobId}><Download size={14} /> {item.state === "partial" ? "Resume" : "Download"}</button>}</div>)}</div>{activeJobId && progress && <><ProgressView progress={progress} /><button className="reset-button danger" onClick={cancelJob}>Cancel</button></>}</section>}
      <section className="intro-row"><h1>Cut out the subject.<br /><span>Keep every detail.</span></h1><p className="intro-copy">Turn images into transparent PNGs and videos into clean green or blue screen footage—entirely on your device.</p></section>
      {!media ? <section className={`drop-zone ${dragging ? "is-dragging" : ""}`} onDragEnter={(event) => { event.preventDefault(); setDragging(true); }} onDragOver={(event) => event.preventDefault()} onDragLeave={(event) => { event.preventDefault(); setDragging(false); }} onDrop={(event) => { event.preventDefault(); setDragging(false); acceptFile(event.dataTransfer.files[0]); }}><div className="drop-glow" /><span className="upload-icon"><UploadCloud size={30} /></span><h2>Drop an image or video here</h2><p>or choose a file from your computer</p><button className="primary-button" onClick={browseFiles}><FolderOpen size={17} /> Browse files</button><div className="format-row"><span><FileImage size={14} /> PNG, JPG, WEBP</span><i /><span><Film size={14} /> MP4, MOV, WEBM</span></div></section> :
      <section className="editor-grid"><div className="preview-panel panel"><div className="panel-heading"><div><span className="media-badge">{media.kind === "image" ? <ImageIcon size={14} /> : <Film size={14} />}</span><h2>{media.name}</h2><p>{formatBytes(media.size)} · {status === "done" ? "Result ready" : "Ready to process"}</p></div><div className="preview-actions"><div className="preview-toggle"><button className={previewMode === "input" ? "active" : ""} onClick={() => setPreviewMode("input")}>Input</button><button className={previewMode === "output" ? "active" : ""} onClick={() => setPreviewMode("output")} disabled={!displayResult}>Output</button></div><button className="icon-button" onClick={reset}><X size={18} /></button></div></div>
        <div className={`media-stage ${media.kind === "image" ? "checkerboard" : previewMode === "output" ? `screen-${screenColor}` : ""}`}>{media.kind === "image" ? <img src={previewSource} alt={previewMode === "output" ? "Background removed" : "Input"} /> : <video key={previewSource} src={previewSource} controls preload="metadata" onTimeUpdate={(event) => { if (previewMode === "input") { playheadRef.current = event.currentTarget.currentTime; setPlayhead(event.currentTarget.currentTime); } }} />}{status === "processing" && <div className="processing-overlay"><span className="spinner" /><strong>{progress?.message ?? "Preparing the cutout"}</strong>{progress && <ProgressView progress={progress} />}{activeJobId && <button className="cancel-overlay" onClick={cancelJob}><Pause size={14} /> Cancel</button>}</div>}{status === "done" && previewMode === "output" && <div className="done-badge"><Check size={15} /> {displayResult?.preview ? "1-second preview" : media.kind === "image" ? "Background removed" : "Full export"}</div>}</div></div>
        <aside className="controls-panel panel"><div className="controls-title"><div><p>OUTPUT SETTINGS</p><h2>Configure cutout</h2></div><Settings2 size={19} /></div>
          <label className="control-group"><span>Detection model</span><div className="segmented three">{(["Auto", "General", "Anime"] as Model[]).map((item) => <button key={item} className={model === item ? "active" : ""} onClick={() => setModel(item)}>{item}</button>)}</div><small>{model === "Anime" ? "Optimized for line art and stylized edges." : "General handles people, animals, products and objects."}</small></label>
          {media.kind === "video" && <label className="control-group"><span>Screen colour</span><div className="color-options"><button className={screenColor === "green" ? "selected" : ""} onClick={() => setScreenColor("green")}><i className="green-swatch" /><span>Green</span>{screenColor === "green" && <Check size={14} />}</button><button className={screenColor === "blue" ? "selected" : ""} onClick={() => setScreenColor("blue")}><i className="blue-swatch" /><span>Blue</span>{screenColor === "blue" && <Check size={14} />}</button></div><small>Motion-aware stabilization reduces mask flicker and resets at scene cuts.</small></label>}
          <label className="control-group"><span>Quality</span><div className="quality-select"><Zap size={16} /><select value={quality} onChange={(event) => setQuality(event.target.value as Quality)}><option>Fast</option><option>Balanced</option><option>Maximum</option></select></div>{requiredModel() && !requiredModel()?.installed && <small className="download-needed">{requiredModel()?.name} needs to be downloaded. <button onClick={() => setShowModels(true)}>Open models</button></small>}</label>
          <label className="control-group range-group"><span><b>Edge detail</b><output>{edgeDetail}%</output></span><input type="range" min="0" max="100" value={edgeDetail} onChange={(event) => setEdgeDetail(Number(event.target.value))} /><small>Preserves fine hair, fur and soft edges.</small></label>
          <div className="export-card"><span>{media.kind === "image" ? <FileImage size={20} /> : <Film size={20} />}</span><div><small>EXPORT FORMAT</small><strong>{media.kind === "image" ? "Transparent PNG" : `${screenColor === "green" ? "Green" : "Blue"} screen MP4`}</strong></div><Check size={16} /></div>
          {media.kind === "video" && <button className="preview-button" onClick={() => runJob(true)} disabled={status === "processing" || !!(requiredModel(true) && !requiredModel(true)?.installed)}><Film size={17} /> Preview 1 second from {Math.floor(playhead)}s</button>}
          <button className="process-button" onClick={fullResult ? saveResult : () => runJob(false)} disabled={processingDisabled}>{fullResult ? <><Download size={18} />{savedPath ? "Save another copy" : `Save ${media.kind === "image" ? "PNG" : "full MP4"}`}</> : status === "processing" ? <><span className="mini-spinner" /> Processing…</> : <><WandSparkles size={18} /> {media.kind === "video" ? "Process full video" : "Remove background"}<ArrowRight size={17} /></>}</button>
          {displayResult && <p className="result-note">{displayResult.preview ? "Temporary preview" : "Full result"} · {(displayResult.durationMs / 1000).toFixed(1)}s · {displayResult.provider.replace("ExecutionProvider", "")}{displayResult.frameCount ? ` · ${displayResult.frameCount} frames` : ""}</p>}
          {fullResult && <button className="reset-button" onClick={() => runJob(false)}><WandSparkles size={14} /> Process again</button>}<button className="reset-button" onClick={reset}><RotateCcw size={14} /> Choose another file</button>
        </aside></section>}
      {error && <div className="error-toast"><X size={15} /> {error}<button onClick={() => setError(null)}><X size={13} /></button></div>}
    </main>
  </div>;
}

function ProgressView({ progress }: { progress: JobProgress }) {
  return <div className="job-progress"><div className={`progress-track ${progress.percent == null ? "indeterminate" : ""}`}><i style={progress.percent == null ? undefined : { width: `${progress.percent}%` }} /></div><small>{progress.percent == null ? progress.message : `${Math.round(progress.percent)}%`}{progress.etaSeconds != null ? ` · about ${progress.etaSeconds}s left` : ""}</small></div>;
}

export default App;
