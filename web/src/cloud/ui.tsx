import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import type { Resource } from "./model";
import { resourcePath } from "./navigation";
export function Icon({ name, size = 18 }: { name: string; size?: number }) {
  const paths: Record<string, ReactNode> = {
    layers: (
      <>
        <path d="m12 3 9 5-9 5-9-5 9-5Z" />
        <path d="m3 12 9 5 9-5M3 16l9 5 9-5" />
      </>
    ),
    radio: (
      <>
        <circle cx="12" cy="12" r="2" />
        <path d="M7 7a7 7 0 0 0 0 10M17 7a7 7 0 0 1 0 10M4 4a11 11 0 0 0 0 16M20 4a11 11 0 0 1 0 16" />
      </>
    ),
    code: <path d="m8 7-5 5 5 5m8-10 5 5-5 5m-3-13-2 16" />,
    route: (
      <>
        <circle cx="5" cy="5" r="2" />
        <circle cx="19" cy="19" r="2" />
        <path d="M7 5h9a4 4 0 0 1 0 8H8a4 4 0 0 0 0 8h9" />
      </>
    ),
    monitor: (
      <>
        <rect x="3" y="4" width="18" height="13" rx="2" />
        <path d="M12 17v4M8 21h8" />
      </>
    ),
    key: (
      <>
        <circle cx="8" cy="8" r="5" />
        <path d="m12 12 9 9m-5-5 3-3m0 6 3-3" />
      </>
    ),
    plus: <path d="M12 5v14M5 12h14" />,
    arrow: <path d="M5 12h14m-5-5 5 5-5 5" />,
    back: <path d="M19 12H5m5-5-5 5 5 5" />,
    search: (
      <>
        <circle cx="10" cy="10" r="6" />
        <path d="m15 15 6 6" />
      </>
    ),
    refresh: (
      <path d="M20 7v5h-5M4 17v-5h5M5 8a8 8 0 0 1 13-3l2 3M4 16l2 3a8 8 0 0 0 13-3" />
    ),
    check: <path d="m5 12 4 4L19 6" />,
    copy: (
      <>
        <rect x="8" y="8" width="12" height="13" rx="2" />
        <path d="M16 8V3H3v13h5" />
      </>
    ),
    close: <path d="m6 6 12 12M6 18 18 6" />,
    menu: <path d="M4 6h16M4 12h16M4 18h16" />,
    clock: (
      <>
        <circle cx="12" cy="12" r="9" />
        <path d="M12 7v5l3 2" />
      </>
    ),
    edit: <path d="m16 3 5 5-12 12-6 1 1-6L16 3Zm-3 3 5 5" />,
    shield: (
      <>
        <path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z" />
        <path d="m8 12 3 3 5-6" />
      </>
    ),
  };
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {paths[name] ?? paths.layers}
    </svg>
  );
}
export function Status({ r }: { r: Resource }) {
  const state =
    r.kind === "bundle"
      ? r.data.error
        ? "error"
        : r.data.published_revision
          ? "ok"
          : "pending"
      : r.kind === "profile" && r.data.type === "source"
        ? r.data.fetch_status
        : r.kind === "device"
          ? r.data.reported?.status
          : "ready";
  const name =
    r.kind === "bundle"
      ? state === "ok"
        ? "已发布"
        : state === "error"
          ? "发布异常"
          : "待发布"
      : r.kind === "device"
        ? ({ applied: "已应用", failed: "应用失败", online: "已上报" }[
            state ?? ""
          ] ?? "未上报")
        : ({ ok: "更新成功", error: "更新失败", ready: "可用" }[state ?? ""] ??
          "等待更新");
  return (
    <span
      className={`status ${state === "error" || state === "failed" ? "bad" : state === "ok" || state === "applied" || state === "ready" ? "good" : "neutral"}`}
    >
      <i />
      {name}
    </span>
  );
}
export function Copy({
  value,
  label = "复制地址",
}: {
  value: string;
  label?: string;
}) {
  const [state, setState] = useState("");
  useEffect(() => {
    if (!state) return;
    const t = setTimeout(() => setState(""), 2200);
    return () => clearTimeout(t);
  }, [state]);
  return (
    <button
      type="button"
      className="copy-button"
      onClick={() => {
        void navigator.clipboard
          .writeText(value)
          .then(() => setState("已复制"))
          .catch(() => setState("复制失败，请手动复制"));
      }}
    >
      <Icon name={state === "已复制" ? "check" : "copy"} size={15} />
      {state || label}
    </button>
  );
}
export function Modal({
  title,
  close,
  children,
}: {
  title: string;
  close: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const node = ref.current;
    node?.showModal();
    return () => node?.close();
  }, []);
  return (
    <dialog ref={ref} className="modal" onCancel={close} aria-label={title}>
      <header>
        <h2>{title}</h2>
        <button className="icon-button" aria-label="关闭" onClick={close}>
          <Icon name="close" />
        </button>
      </header>
      {children}
    </dialog>
  );
}
export function Empty({
  title = "这里还没有内容",
  text,
  action,
}: {
  title?: string;
  text: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <span className="empty-icon">
        <Icon name="layers" size={28} />
      </span>
      <h3>{title}</h3>
      <p>{text}</p>
      {action}
    </div>
  );
}
export function CodeBlock({ content }: { content: string }) {
  const lines = content.split("\n");
  const [showPreamble, setShowPreamble] = useState(false);
  const firstConfigLine = lines.findIndex(
    (line) => line.trim() !== "" && !line.trim().startsWith("#"),
  );
  const hasPreamble = firstConfigLine > 8;
  const offset = hasPreamble && !showPreamble ? firstConfigLine : 0;
  return (
    <>
      <div className="code-toolbar">
        <span>
          YAML <b>·</b> {lines.length.toLocaleString()} 行
        </span>
        {hasPreamble && (
          <button
            type="button"
            className="copy-button"
            aria-expanded={showPreamble}
            onClick={() => setShowPreamble(!showPreamble)}
          >
            {showPreamble
              ? "折叠开头注释"
              : `展开来源与许可 · ${firstConfigLine} 行`}
          </button>
        )}
        <Copy value={content} label="复制内容" />
      </div>
      <div className="code-view" tabIndex={0} aria-label="配置内容">
        <ol start={offset + 1}>
          {lines.slice(offset, offset + 1000).map((line, i) => (
            <li key={i}>
              <code className={line.trim().startsWith("#") ? "comment" : ""}>
                {line || " "}
              </code>
            </li>
          ))}
        </ol>
      </div>
      {lines.length - offset > 1000 && (
        <p className="muted">
          最多预览 1,000 行。复制内容可获取包含全部注释的完整配置。
        </p>
      )}
    </>
  );
}
/** Shared detail surface: one header inset, explicit body or edge-to-edge code. */
export function Panel({
  title,
  description,
  actions,
  children,
  className = "",
}: {
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`panel ${className}`}>
      <div className="panel-heading">
        <div>
          <h2>{title}</h2>
          {description && <p className="muted">{description}</p>}
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}
export function PanelBody({
  children,
  className = "",
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={`panel-body ${className}`}>{children}</div>;
}
export function FieldActionRow({ children }: { children: ReactNode }) {
  return <div className="field-action-row">{children}</div>;
}
/** All YAML viewing surfaces share toolbar, copy, line numbers and comment treatment. */
export function ConfigPreview({
  title = "YAML 配置",
  description,
  actions,
  controls,
  content,
  loading,
  error,
  warnings = [],
  compatibility = [],
  empty = "生成预览后，可查看完整 YAML。",
}: {
  title?: string;
  description?: ReactNode;
  actions?: ReactNode;
  controls?: ReactNode;
  content?: string;
  loading?: boolean;
  error?: string;
  warnings?: string[];
  compatibility?: { target: string; message: string }[];
  empty?: string;
}) {
  return (
    <Panel
      title={title}
      description={description}
      actions={actions}
      className="preview-panel config-preview"
    >
      {controls}
      {warnings.length > 0 && (
        <div className="config-notices" role="status">
          <strong>请检查规则与策略</strong>
          <ul>
            {warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}
      {compatibility.length > 0 && (
        <details className="config-notices">
          <summary>{compatibility.length} 种输出格式暂不可用</summary>
          <p>不影响下方成功生成的 YAML；使用对应客户端前需处理这些兼容问题。</p>
          <ul>
            {compatibility.map((v) => (
              <li key={v.target}>
                <strong>{v.target}</strong>
                <span>{v.message}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
      {error ? (
        <p role="alert" className="inline-error">
          {error}
        </p>
      ) : loading ? (
        <p className="panel-message" role="status">
          正在生成预览…
        </p>
      ) : content !== undefined ? (
        <CodeBlock content={content} />
      ) : (
        <p className="panel-message">{empty}</p>
      )}
    </Panel>
  );
}
export function ResourceLink({ r }: { r?: Resource }) {
  return r ? (
    <Link className="text-link" to={resourcePath(r)}>
      {r.data.name}
      <Icon name="arrow" size={13} />
    </Link>
  ) : (
    <span className="muted">未关联</span>
  );
}
