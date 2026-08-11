import { useState, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import { ApiRequestError } from "../../api/client";
import { useCreatePost } from "../../api/hooks";
import { Button } from "../Button/Button";
import { Input } from "../Input/Input";
import { Select } from "../Select/Select";
import { ErrorState } from "../Status/Status";
import "./post-composer.css";

const KINDS = [
  { value: "article", label: "Article" },
  { value: "post", label: "Post" },
  { value: "question", label: "Question" },
  { value: "poll", label: "Poll" },
];

const VISIBILITY = [
  { value: "public", label: "Public — everyone can see it" },
  { value: "unlisted", label: "Unlisted — link only" },
  { value: "private", label: "Private — only you" },
];

/** Compose + publish a new post. Inline in the My feed page. */
export function PostComposer() {
  const navigate = useNavigate();
  const [kind, setKind] = useState<string>("article");
  const [title, setTitle] = useState("");
  const [summary, setSummary] = useState("");
  const [body, setBody] = useState("");
  const [coverImageUrl, setCoverImageUrl] = useState("");
  const [visibility, setVisibility] = useState<string>("public");
  const [tags, setTags] = useState("");
  const [preview, setPreview] = useState(false);
  const createPost = useCreatePost({
    onSuccess: (data) => {
      const id = (data as { id?: string; post?: { id?: string } } | null)?.id
        ?? (data as { post?: { id?: string } } | null)?.post?.id;
      if (id) navigate(`/posts/${id}`);
    },
  });

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    createPost.mutate({
      kind,
      title: title.trim() || undefined,
      summary: summary.trim() || null,
      body: body.trim(),
      cover_image_url: coverImageUrl.trim() || null,
      visibility,
      tags: tags
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
    });
  }

  const error =
    createPost.error instanceof ApiRequestError
      ? createPost.error.detail ?? createPost.error.message
      : null;

  return (
    <form className="composer" onSubmit={onSubmit}>
      <header className="composer__header">
        <h2 className="composer__title">Write a post</h2>
      </header>

      <div className="composer__row">
        <Select label="Kind" value={kind} onValueChange={setKind} options={KINDS} />
        <Select
          label="Visibility"
          value={visibility}
          onValueChange={setVisibility}
          options={VISIBILITY}
        />
      </div>

      {kind !== "post" && (
        <Input
          id="composer-title"
          label="Title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder={kind === "question" ? "What would you like to ask?" : "A clear, specific title"}
          required={kind !== "post"}
        />
      )}

      <Input
        id="composer-summary"
        label="Summary"
        value={summary}
        onChange={(e) => setSummary(e.target.value)}
        placeholder="One or two sentences (shown in feeds)"
      />

      <div className="composer__body">
        <div className="composer__body-head">
          <label className="composer__label" htmlFor="composer-body">
            Body
          </label>
          <button
            type="button"
            className="composer__preview-toggle"
            onClick={() => setPreview((p) => !p)}
            aria-pressed={preview}
          >
            {preview ? "Edit" : "Preview"}
          </button>
        </div>
        {preview ? (
          <div className="composer__preview">{renderPreview(body)}</div>
        ) : (
          <textarea
            id="composer-body"
            className="composer__textarea"
            rows={10}
            value={body}
            onChange={(e) => setBody(e.target.value)}
            placeholder="Write in Markdown — headings, lists, code blocks all supported."
          />
        )}
      </div>

      <Input
        id="composer-cover"
        label="Cover image URL (optional)"
        type="url"
        value={coverImageUrl}
        onChange={(e) => setCoverImageUrl(e.target.value)}
        placeholder="https://…"
      />
      <Input
        id="composer-tags"
        label="Tags (comma separated)"
        value={tags}
        onChange={(e) => setTags(e.target.value)}
        placeholder="rust, async, observability"
      />

      {error && <ErrorState title="Couldn't publish" message={error} />}

      <div className="composer__actions">
        <Button type="submit" loading={createPost.isPending} disabled={body.trim().length === 0}>
          {createPost.isPending ? "Publishing…" : "Publish"}
        </Button>
      </div>
    </form>
  );
}

/** Minimal, safe markdown preview — block-level rendering only, no raw HTML. */
function renderPreview(markdown: string): React.ReactNode {
  const lines = markdown.split("\n");
  return (
    <div className="composer__preview-body">
      {lines.map((line, i) => {
        if (line.startsWith("### ")) return <h4 key={i}>{line.slice(4)}</h4>;
        if (line.startsWith("## ")) return <h3 key={i}>{line.slice(3)}</h3>;
        if (line.startsWith("# ")) return <h2 key={i}>{line.slice(2)}</h2>;
        if (line.startsWith("- ") || line.startsWith("* "))
          return <li key={i}>{line.slice(2)}</li>;
        if (line.trim() === "") return <br key={i} />;
        return <p key={i}>{line}</p>;
      })}
    </div>
  );
}
