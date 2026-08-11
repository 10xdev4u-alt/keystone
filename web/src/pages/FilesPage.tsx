import { useRef, useState, type FormEvent } from "react";
import { useMyFiles, useUploadFile } from "../api/hooks";
import { Button } from "../components/Button/Button";
import { EmptyState, ErrorState, Skeleton } from "../components/Status/Status";
import "./files.css";

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

/** My files — the /me/files placeholder, now real: upload + list. */
export function FilesPage() {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [selected, setSelected] = useState<File | null>(null);
  const { data, isLoading, isError, error, refetch } = useMyFiles();
  const upload = useUploadFile({
    onSuccess: () => {
      setSelected(null);
      if (inputRef.current) inputRef.current.value = "";
    },
  });

  const items = data?.items ?? [];

  function onFileChosen(file: File | null) {
    if (file && file.size > 0) setSelected(file);
  }

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (!selected) return;
    upload.mutate({ file: selected });
  }

  const uploadError = upload.error instanceof Error ? upload.error.message : null;

  return (
    <div className="files">
      <header className="files__header">
        <div>
          <h1 className="files__title">My files</h1>
          <p className="files__subtitle">Uploads live in your private storage (1 GiB quota).</p>
        </div>
      </header>

      <form className="files__upload" onSubmit={onSubmit}>
        <input
          ref={inputRef}
          id="file-input"
          type="file"
          className="files__input"
          aria-label="Choose a file to upload"
          onChange={(e) => onFileChosen(e.target.files?.[0] ?? null)}
        />
        <Button type="submit" disabled={!selected} loading={upload.isPending}>
          {upload.isPending ? "Uploading…" : "Upload"}
        </Button>
        {uploadError && <p className="files__error" role="alert">{uploadError}</p>}
      </form>

      {isLoading ? (
        <div className="files__list" data-testid="files-loading" aria-label="Loading files">
          {[0, 1, 2].map((i) => (
            <div className="file-row file-row--skeleton" key={i}>
              <Skeleton className="file-row__name" />
            </div>
          ))}
        </div>
      ) : isError ? (
        <ErrorState
          title="Couldn't load files"
          message={error instanceof Error ? error.message : undefined}
          onRetry={() => void refetch()}
        />
      ) : items.length === 0 ? (
        <EmptyState
          headingLevel={2}
          title="No files yet"
          description="Upload your first file above — it will appear here."
        />
      ) : (
        <ul className="files__list">
          {items.map((file) => (
            <li key={file.id} className="file-row">
              <span className="file-row__icon" aria-hidden="true">📄</span>
              <span className="file-row__name">{file.original_name}</span>
              <span className="file-row__meta">
                {formatBytes(file.size_bytes)}
                {file.width ? ` · ${file.width}×${file.height}` : ""}
              </span>
              <span className="file-row__date">{formatDate(file.created_at)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
