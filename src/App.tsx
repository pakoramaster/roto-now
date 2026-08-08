import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowRight,
  Check,
  ChevronDown,
  CircleHelp,
  Download,
  FileImage,
  Film,
  FolderOpen,
  Image as ImageIcon,
  Layers3,
  Moon,
  RotateCcw,
  Settings2,
  UploadCloud,
  WandSparkles,
  X,
  Zap,
} from "lucide-react";

type MediaKind = "image" | "video";
type Quality = "Fast" | "Balanced" | "Maximum";
type Model = "Auto" | "General" | "Anime";
type ScreenColor = "green" | "blue";
type PreviewMode = "input" | "output";

interface ImportedMedia {
  file?: File;
  path?: string;
  name: string;
  size: number;
  kind: MediaKind;
  url: string;
}

interface NativeMediaInfo {
  path: string;
  name: string;
  size: number;
  kind: MediaKind;
  previewDataUrl?: string;
}

interface ProcessResult {
  outputPath: string;
  model: string;
  provider: string;
  durationMs: number;
  previewDataUrl?: string;
  frameCount?: number;
}

const isTauriRuntime = () => "__TAURI_INTERNALS__" in window;

const formatBytes = (bytes: number) => {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};

const isSupported = (file: File) =>
  file.type.startsWith("image/") || file.type.startsWith("video/");

function App() {
  const inputRef = useRef<HTMLInputElement>(null);
  const [media, setMedia] = useState<ImportedMedia | null>(null);
  const [dragging, setDragging] = useState(false);
  const [model, setModel] = useState<Model>("Auto");
  const [quality, setQuality] = useState<Quality>("Balanced");
  const [screenColor, setScreenColor] = useState<ScreenColor>("green");
  const [edgeDetail, setEdgeDetail] = useState(72);
  const [status, setStatus] = useState<"idle" | "ready" | "processing" | "done">("idle");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ProcessResult | null>(null);
  const [previewMode, setPreviewMode] = useState<PreviewMode>("input");
  const [savedPath, setSavedPath] = useState<string | null>(null);

  useEffect(() => () => {
    if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
  }, [media]);

  const discardTemporaryResult = (current: ProcessResult | null) => {
    if (current && isTauriRuntime()) {
      void invoke("discard_output", { path: current.outputPath }).catch(() => undefined);
    }
  };

  const acceptFile = (file?: File) => {
    if (!file) return;
    if (!isSupported(file)) {
      setError("Choose a supported image or video file.");
      return;
    }
    if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
    const kind: MediaKind = file.type.startsWith("video/") ? "video" : "image";
    setMedia({ file, name: file.name, size: file.size, kind, url: URL.createObjectURL(file) });
    discardTemporaryResult(result);
    setResult(null);
    setPreviewMode("input");
    setSavedPath(null);
    setError(null);
    setStatus("ready");
  };

  const browseFiles = async () => {
    if (!isTauriRuntime()) {
      inputRef.current?.click();
      return;
    }
    try {
      const selected = await openDialog({
        multiple: false,
        directory: false,
        filters: [
          { name: "Images and videos", extensions: ["png", "jpg", "jpeg", "webp", "mp4", "mov", "webm"] },
        ],
      });
      if (!selected || Array.isArray(selected)) return;
      const info = await invoke<NativeMediaInfo>("inspect_media", { path: selected });
      if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
      setMedia({
        path: info.path,
        name: info.name,
        size: info.size,
        kind: info.kind,
        url: info.previewDataUrl ?? convertFileSrc(info.path),
      });
      discardTemporaryResult(result);
      setResult(null);
      setPreviewMode("input");
      setSavedPath(null);
      setError(null);
      setStatus("ready");
    } catch (caught) {
      setError(String(caught));
    }
  };

  const reset = () => {
    if (media?.url.startsWith("blob:")) URL.revokeObjectURL(media.url);
    discardTemporaryResult(result);
    setMedia(null);
    setResult(null);
    setPreviewMode("input");
    setSavedPath(null);
    setStatus("idle");
    setError(null);
    if (inputRef.current) inputRef.current.value = "";
  };

  const runPrototype = async () => {
    if (!media || status === "processing") return;
    if (!isTauriRuntime() || !media.path) {
      setStatus("processing");
      window.setTimeout(() => setStatus("done"), 1800);
      return;
    }
    try {
      setError(null);
      discardTemporaryResult(result);
      setResult(null);
      setSavedPath(null);
      setStatus("processing");
      const command = media.kind === "image" ? "process_image" : "process_video";
      const processed = await invoke<ProcessResult>(command, {
        inputPath: media.path,
        model,
        quality,
        edgeDetail,
        ...(media.kind === "video" ? { screenColor } : {}),
      });
      setResult(processed);
      setPreviewMode("output");
      setStatus("done");
    } catch (caught) {
      setStatus("ready");
      setError(String(caught));
    }
  };

  const saveResult = async () => {
    if (!media || !result || !media.path) return;
    try {
      const stem = media.name.replace(/\.[^.]+$/, "");
      const extension = media.kind === "image" ? "png" : "mp4";
      const suffix = media.kind === "image" ? "cutout" : `${screenColor}screen`;
      const defaultPath = media.path.replace(/[^\\/]+$/, `${stem}_${suffix}.${extension}`);
      const destinationPath = await saveDialog({
        defaultPath,
        filters: [{
          name: media.kind === "image" ? "Transparent PNG" : "Screen video",
          extensions: [extension],
        }],
      });
      if (!destinationPath) return;
      const saved = await invoke<string>("save_output", {
        sourcePath: result.outputPath,
        destinationPath,
      });
      setSavedPath(saved);
      setError(null);
    } catch (caught) {
      setError(String(caught));
    }
  };

  const previewSource = previewMode === "output" && result
    ? (media?.kind === "image" ? result.previewDataUrl ?? "" : convertFileSrc(result.outputPath))
    : media?.url ?? "";

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark"><Layers3 size={18} strokeWidth={2.4} /></span>
          <span>ROTO<span className="brand-accent">NOW</span></span>
          <span className="prototype-pill">PROTOTYPE</span>
        </div>
        <nav className="top-actions" aria-label="App actions">
          <button className="icon-button" aria-label="Help"><CircleHelp size={18} /></button>
          <button className="icon-button" aria-label="Toggle theme"><Moon size={18} /></button>
        </nav>
      </header>

      <main className="workspace">
        <input
          ref={inputRef}
          className="visually-hidden"
          type="file"
          accept="image/png,image/jpeg,image/webp,video/mp4,video/quicktime,video/webm"
          onChange={(event) => acceptFile(event.target.files?.[0])}
        />
        <section className="intro-row">
          <div>
            <h1>Cut out the subject.<br /><span>Keep every detail.</span></h1>
            <p className="intro-copy">Turn images into transparent PNGs and videos into clean green or blue screen footage—entirely on your device.</p>
          </div>
        </section>

        {!media ? (
          <section
            className={`drop-zone ${dragging ? "is-dragging" : ""}`}
            onDragEnter={(event) => { event.preventDefault(); setDragging(true); }}
            onDragOver={(event) => event.preventDefault()}
            onDragLeave={(event) => { event.preventDefault(); setDragging(false); }}
            onDrop={(event) => {
              event.preventDefault();
              setDragging(false);
              acceptFile(event.dataTransfer.files[0]);
            }}
          >
            <div className="drop-glow" />
            <span className="upload-icon"><UploadCloud size={30} /></span>
            <h2>Drop an image or video here</h2>
            <p>or choose a file from your computer</p>
            <button className="primary-button" onClick={browseFiles}>
              <FolderOpen size={17} /> Browse files
            </button>
            <div className="format-row">
              <span><FileImage size={14} /> PNG, JPG, WEBP</span>
              <i />
              <span><Film size={14} /> MP4, MOV, WEBM</span>
            </div>
          </section>
        ) : (
          <section className="editor-grid">
            <div className="preview-panel panel">
              <div className="panel-heading">
                <div>
                  <span className="media-badge">{media.kind === "image" ? <ImageIcon size={14} /> : <Film size={14} />}{media.kind}</span>
                  <h2>{media.name}</h2>
                  <p>{formatBytes(media.size)} · {status === "done" ? "Result ready" : "Ready to process"}</p>
                </div>
                <div className="preview-actions">
                  <div className="preview-toggle" aria-label="Preview source">
                    <button className={previewMode === "input" ? "active" : ""} onClick={() => setPreviewMode("input")}>Input</button>
                    <button className={previewMode === "output" ? "active" : ""} onClick={() => setPreviewMode("output")} disabled={!result}>Output</button>
                  </div>
                  <button className="icon-button" onClick={reset} aria-label="Remove file"><X size={18} /></button>
                </div>
              </div>

              <div className={`media-stage ${media.kind === "image" ? "checkerboard" : previewMode === "output" ? `screen-${screenColor}` : ""}`}>
                {media.kind === "image" ? (
                  <img src={previewSource} alt={previewMode === "output" ? "Background removed preview" : "Imported preview"} />
                ) : (
                  previewSource ? <video key={previewSource} src={previewSource} controls preload="metadata" /> : <div className="video-placeholder"><Film size={34} /><strong>{media.name}</strong><small>Preview unavailable for this video.</small></div>
                )}
                {status === "processing" && (
                  <div className="processing-overlay">
                    <span className="spinner" />
                    <strong>Preparing the cutout</strong>
                    <small>{model === "Anime" ? "Running ToonOut locally" : "Running BiRefNet locally"}</small>
                  </div>
                )}
                {status === "done" && previewMode === "output" && (
                  <div className="done-badge"><Check size={15} /> {media.kind === "image" ? "Background removed" : "Video exported"}</div>
                )}
              </div>

            </div>

            <aside className="controls-panel panel">
              <div className="controls-title"><div><p>OUTPUT SETTINGS</p><h2>Configure cutout</h2></div><Settings2 size={19} /></div>

              <label className="control-group">
                <span>Detection model</span>
                <div className="segmented three">
                  {(["Auto", "General", "Anime"] as Model[]).map((item) => (
                    <button key={item} className={model === item ? "active" : ""} onClick={() => setModel(item)}>{item}</button>
                  ))}
                </div>
                <small>{model === "Auto" ? "Uses the general model; choose Anime for stylized artwork." : model === "Anime" ? "Optimized for line art and stylized edges." : "Best for people, animals and objects."}</small>
              </label>

              {media.kind === "video" && (
                <label className="control-group">
                  <span>Screen colour</span>
                  <div className="color-options">
                    <button className={screenColor === "green" ? "selected" : ""} onClick={() => setScreenColor("green")}><i className="green-swatch" /><span>Green</span>{screenColor === "green" && <Check size={14} />}</button>
                    <button className={screenColor === "blue" ? "selected" : ""} onClick={() => setScreenColor("blue")}><i className="blue-swatch" /><span>Blue</span>{screenColor === "blue" && <Check size={14} />}</button>
                  </div>
                </label>
              )}

              <label className="control-group">
                <span>Quality</span>
                <div className="quality-select">
                  <Zap size={16} />
                  <select value={quality} onChange={(event) => setQuality(event.target.value as Quality)}>
                    <option>Fast</option><option>Balanced</option><option>Maximum</option>
                  </select>
                  <ChevronDown size={16} />
                </div>
              </label>

              <label className="control-group range-group">
                <span><b>Edge detail</b><output>{edgeDetail}%</output></span>
                <input type="range" min="0" max="100" value={edgeDetail} onChange={(event) => setEdgeDetail(Number(event.target.value))} />
                <small>Preserves fine hair, fur and soft edges.</small>
              </label>

              <div className="export-card">
                <span>{media.kind === "image" ? <FileImage size={20} /> : <Film size={20} />}</span>
                <div><small>EXPORT FORMAT</small><strong>{media.kind === "image" ? "Transparent PNG" : `${screenColor === "green" ? "Green" : "Blue"} screen MP4`}</strong></div>
                <Check size={16} />
              </div>

              <button className="process-button" onClick={result ? saveResult : runPrototype} disabled={status === "processing"}>
                {result ? <><Download size={18} /> {savedPath ? "Save another copy" : `Save ${media.kind === "image" ? "PNG" : "MP4"}`}</> : status === "processing" ? <><span className="mini-spinner" /> Removing background…</> : <><WandSparkles size={18} /> Remove background <ArrowRight size={17} /></>}
              </button>
              {result && <p className="result-note">Processed in {(result.durationMs / 1000).toFixed(1)}s using {result.provider.replace("ExecutionProvider", "")}{result.frameCount ? ` · ${result.frameCount} frames` : ""}{savedPath ? " · Saved" : ""}</p>}
              {result && <button className="reset-button" onClick={runPrototype}><WandSparkles size={14} /> Process again</button>}
              <button className="reset-button" onClick={reset}><RotateCcw size={14} /> Choose another file</button>
            </aside>
          </section>
        )}

        {error && <div className="error-toast"><X size={15} /> {error}</div>}

      </main>
    </div>
  );
}

export default App;
