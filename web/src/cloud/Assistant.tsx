import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Resource } from "./model";
import { ConfigPreview, FieldActionRow, Icon, Panel, PanelBody } from "./ui";
import { useWorkspace } from "./context";
import "./assistant.css";

type Config = { enabled: boolean; model: string; effort: string; portal_url?: string };
type Event = { type: "user" | "assistant" | "tool"; text?: string; name?: string; draft_id?: string; error?: string; lines?: number };
type Session = { id: string; current_draft?: string; events: Event[] };
type Draft = {
  id: string;
  status: string;
  original: string;
  content: string;
  stale: boolean;
  affected: { id: string; name: string }[];
  errors: { identity_id?: string; name?: string; error: string }[];
  validation_hash: string;
  can_commit: boolean;
};

const sessionKey = (profileId: string) => `camofy-assistant-session:${profileId}`;

async function streamTurn(id: string, text: string, onEvent: (value: Record<string, unknown>) => void) {
  const response = await fetch(`/api/assistant/sessions/${id}/turn`, {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error ?? `HTTP ${response.status}`);
  }
  if (!response.body) throw new Error("服务器没有返回消息流");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    buffer = buffer.replace(/\r\n/g, "\n");
    const chunks = buffer.split("\n\n");
    buffer = chunks.pop() ?? "";
    for (const chunk of chunks) {
      const line = chunk.split("\n").find((part) => part.startsWith("data: "));
      if (line) onEvent(JSON.parse(line.slice(6)));
    }
  }
}

function changeCount(before: string, after: string) {
  const left = before.split("\n");
  const right = after.split("\n");
  const common = Math.min(left.length, right.length);
  let changed = Math.abs(left.length - right.length);
  for (let i = 0; i < common; i++) if (left[i] !== right[i]) changed++;
  return changed;
}

export function AssistantEditor({ profile }: { profile: Resource }) {
  const { load } = useWorkspace();
  const [config, setConfig] = useState<Config>();
  const [session, setSession] = useState<Session>();
  const [draft, setDraft] = useState<Draft>();
  const [message, setMessage] = useState("");
  const [streamText, setStreamText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const chatArea = useRef<HTMLDivElement>(null);
  const getDraft = useCallback(async (id: string) => {
    const value = await api<Draft>(`/assistant/drafts/${id}`);
    setDraft(value);
  }, []);
  const getSession = useCallback(async (id: string) => {
    const value = await api<Session>(`/assistant/sessions/${id}`);
    setSession(value);
    if (value.current_draft) await getDraft(value.current_draft);
  }, [getDraft]);

  useEffect(() => {
    let active = true;
    void api<Config>("/assistant/config").then((value) => { if (active) setConfig(value); }).catch(() => { if (active) setConfig({ enabled: false, model: "gpt-6-luna", effort: "medium" }); });
    const saved = localStorage.getItem(sessionKey(profile.id));
    if (saved) void getSession(saved).catch(() => { if (active) localStorage.removeItem(sessionKey(profile.id)); });
    return () => { active = false; };
  }, [profile.id, getSession]);
  useEffect(() => { if (chatArea.current) chatArea.current.scrollTop = chatArea.current.scrollHeight; }, [session?.events.length, streamText]);

  async function ensureSession() {
    if (session) return session.id;
    const created = await api<{ id: string }>(`/profiles/${profile.id}/assistant/sessions`, "POST");
    localStorage.setItem(sessionKey(profile.id), created.id);
    setSession({ id: created.id, events: [] });
    return created.id;
  }
  async function send() {
    const text = message.trim();
    if (!text || busy) return;
    setBusy(true); setError(""); setMessage(""); setStreamText("");
    try {
      const id = await ensureSession();
      setSession((old) => old && ({ ...old, events: [...old.events, { type: "user", text }] }));
      let draftId: string | undefined;
      await streamTurn(id, text, (value) => {
        if (value.type === "delta") setStreamText((s) => s + String(value.text ?? ""));
        if (value.type === "tool" && typeof value.draft_id === "string") draftId = value.draft_id;
        if (value.type === "error") setError(String(value.message ?? "AI 请求失败"));
      });
      await getSession(id);
      if (draftId) await getDraft(draftId);
      setStreamText("");
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  }
  async function commit() {
    if (!draft?.can_commit || busy) return;
    if (!window.confirm(`提交这份草稿并发布到 ${draft.affected.length} 个身份？请确认下方差异。`)) return;
    setBusy(true); setError("");
    try {
      await api(`/assistant/drafts/${draft.id}/commit`, "POST", { validation_hash: draft.validation_hash });
      await Promise.all([getDraft(draft.id), load()]);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  }
  function newConversation() {
    localStorage.removeItem(sessionKey(profile.id)); setSession(undefined); setDraft(undefined); setStreamText(""); setError("");
  }

  return <div className="assistant-layout">
    <div className="assistant-main">
      <Panel title="AI 编辑" description="让 AI 阅读指定行、精确生成草稿；提交前始终由你审核。"
        actions={<div className="assistant-model"><span>GPT 6 Luna · medium</span>{config?.portal_url && <a href={config.portal_url} target="_blank" rel="noopener noreferrer">更多模型 ↗</a>}</div>}>
        <PanelBody>
          <p className="assistant-privacy">你选择的配置片段会发送至已配置的 AI 服务。密码、令牌和订阅 URL 等已知字段会遮蔽，AI 不能修改这些行；如有其他敏感信息，请先手动移除。</p>
          {!config?.enabled && config && <div className="banner error" role="alert">AI 服务尚未配置，请联系管理员。</div>}
          <div className="assistant-chat" aria-live="polite" ref={chatArea}>
            {!session?.events.length && <div className="assistant-empty"><Icon name="edit" size={24}/><strong>描述你想修改的内容</strong><span>例如“把指定域名改为 DIRECT，并保持其余规则不变”。</span></div>}
            {session?.events.map((entry, index) => entry.type === "tool"
              ? <div className="assistant-tool" key={index}>{entry.name === "profile_read" ? `已读取 ${entry.lines ?? 0} 行` : entry.name === "profile_replace" ? "已生成草稿，尚未生效" : "等待你的确认"}{entry.error && <span> · {entry.error}</span>}</div>
              : <div key={index} className={`assistant-message ${entry.type}`}><span>{entry.type === "user" ? "你" : "Camofy Agent"}</span><p>{entry.text}</p></div>)}
            {streamText && <div className="assistant-message assistant"><span>Camofy Agent · 正在生成</span><p>{streamText}</p></div>}
          </div>
          {error && <div className="assistant-error" role="alert">{error}</div>}
          <form className="assistant-compose" onSubmit={(e) => { e.preventDefault(); void send(); }}>
            <label htmlFor="assistant-request">编辑要求</label>
            <textarea id="assistant-request" value={message} onChange={(e) => setMessage(e.target.value)} rows={3} maxLength={4000} placeholder="描述要改哪段配置，以及预期结果…" disabled={busy || !config?.enabled}/>
            <FieldActionRow><button type="submit" className="primary" disabled={busy || !message.trim() || !config?.enabled}>{busy ? "正在处理…" : "发送要求"}</button><button type="button" className="quiet" onClick={newConversation} disabled={busy}>新对话</button></FieldActionRow>
          </form>
        </PanelBody>
      </Panel>
    </div>
    <aside className="assistant-review">
      {draft ? <>
        <Panel title="草稿审核" description={draft.status === "committed" ? "已提交到云端；设备生效状态以设备回报为准。" : `草稿未生效 · ${changeCount(draft.original, draft.content)} 行发生变化`}
          actions={<span className="chip">{draft.status === "committed" ? "已提交" : draft.stale ? "需要重建" : "待审核"}</span>}>
          <PanelBody>
            {draft.errors.length > 0 && <div className="assistant-validation" role="alert"><strong>配置校验未通过</strong>{draft.errors.map((e, index) => <p key={index}>{e.name ? `${e.name}：` : ""}{e.error}</p>)}</div>}
            {draft.stale && <div className="assistant-validation" role="alert">正式 Profile 已发生变化，请重新读取并生成草稿。</div>}
            <div className="assistant-affected"><strong>影响范围</strong>{draft.affected.length ? <ul>{draft.affected.map((item) => <li key={item.id}>{item.name}</li>)}</ul> : <p>尚未关联已启用的身份；仅验证配置片段。</p>}</div>
            <FieldActionRow><button className="primary" disabled={!draft.can_commit || busy} onClick={() => void commit()}>确认提交</button><span className="muted">不会直接证明设备已应用</span></FieldActionRow>
          </PanelBody>
        </Panel>
        <div className="assistant-diff"><ConfigPreview title="修改前" content={draft.original}/><ConfigPreview title="草稿内容" content={draft.content}/></div>
      </> : <Panel title="草稿审核" description="AI 生成草稿后，变更和受影响身份会出现在这里。"><PanelBody><p className="muted">正式配置不会因对话或生成草稿而改变。</p></PanelBody></Panel>}
    </aside>
  </div>;
}
