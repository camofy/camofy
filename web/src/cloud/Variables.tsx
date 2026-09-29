import { useState } from "react";
import type {
  ExportCandidate,
  IdentityValue,
  Resource,
  TemplateVariable,
  VariableBinding,
  VariableExport,
  VariableSelector,
  VariableType,
} from "./model";

const types: { value: VariableType; label: string }[] = [
  { value: "string", label: "文本" },
  { value: "outbound", label: "节点 / 策略组引用" },
  { value: "integer", label: "整数" },
  { value: "number", label: "数字" },
  { value: "boolean", label: "布尔值" },
  { value: "list", label: "列表" },
  { value: "object", label: "对象" },
];

function TypeSelect({ value, onChange }: { value: VariableType; onChange: (value: VariableType) => void }) {
  return <select aria-label="变量类型" value={value} onChange={(e) => onChange(e.target.value as VariableType)}>
    {types.map((type) => <option key={type.value} value={type.value}>{type.label}</option>)}
  </select>;
}

function LiteralInput({ value, type, onChange }: { value: unknown; type: VariableType; onChange: (value: unknown) => void }) {
  const [draft, setDraft] = useState(() => value === undefined ? "" : typeof value === "string" ? value : JSON.stringify(value));
  const [invalid, setInvalid] = useState(false);
  if (type === "boolean") return <select aria-label="布尔值" value={value === true ? "true" : "false"} onChange={(e) => onChange(e.target.value === "true")}>
    <option value="false">否</option><option value="true">是</option>
  </select>;
  if (type === "string" || type === "outbound") return <input aria-label="变量值" value={typeof value === "string" ? value : ""} onChange={(e) => onChange(e.target.value)} placeholder={type === "outbound" ? "精确的节点或策略组名称" : "输入值"} />;
  const commit = (text: string) => {
    try {
      const parsed = JSON.parse(text);
      if ((type === "integer" && !Number.isInteger(parsed)) ||
        (type === "number" && typeof parsed !== "number") ||
        (type === "list" && !Array.isArray(parsed)) ||
        (type === "object" && (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)))) throw new Error("type");
      onChange(parsed);
      setInvalid(false);
    } catch { setInvalid(true); }
  };
  return <span className="variable-value-field">
    <input aria-label="JSON 变量值" aria-invalid={invalid} value={draft} onChange={(e) => { setDraft(e.target.value); commit(e.target.value); }} onBlur={() => commit(draft)} placeholder={type === "list" ? "[\"a\", \"b\"]" : type === "object" ? "{\"key\": \"value\"}" : "123"} />
    {invalid && <small className="inline-error">请输入符合类型的 JSON 值</small>}
  </span>;
}

export function VariableEditor({ value, onChange }: { value: TemplateVariable[]; onChange: (value: TemplateVariable[]) => void }) {
  const update = (index: number, change: Partial<TemplateVariable>) => onChange(value.map((item, i) => i === index ? { ...item, ...change } : item));
  return <section className="capability-editor">
    <div className="section-heading"><div><h3>输入变量</h3><p className="muted">作者在 YAML 的值位置写 <code>{"{{camofy.变量名}}"}</code>；身份决定填入什么。不限于节点或某个字段。</p></div>
      <button type="button" onClick={() => onChange([...value, { key: "", label: "", type: "string", required: true }])}>添加变量</button>
    </div>
    {value.map((item, index) => <div className="variable-contract-row" key={index}>
      <label>变量名<input required pattern="(?:[A-Za-z0-9_]|-){1,64}" maxLength={64} value={item.key} onChange={(e) => update(index, { key: e.target.value })} placeholder="例如 upstream" /></label>
      <label>显示名称<input required maxLength={120} value={item.label} onChange={(e) => update(index, { label: e.target.value })} placeholder="例如前置代理" /></label>
      <label>类型<TypeSelect value={item.type} onChange={(type) => update(index, { type, default: undefined })} /></label>
      <label className="check"><input type="checkbox" checked={item.required} onChange={(e) => update(index, { required: e.target.checked })} />必填</label>
      <label className="check"><input type="checkbox" checked={item.default !== undefined} onChange={(e) => update(index, { default: e.target.checked ? item.type === "boolean" ? false : item.type === "integer" || item.type === "number" ? 0 : item.type === "list" ? [] : item.type === "object" ? {} : "" : undefined })} />提供默认值</label>
      {item.default !== undefined && <label>默认值<LiteralInput key={`${index}-${item.type}`} type={item.type} value={item.default} onChange={(defaultValue) => update(index, { default: defaultValue })} /></label>}
      <button type="button" className="danger" onClick={() => onChange(value.filter((_, i) => i !== index))}>移除</button>
      <small className="variable-contract-hint">YAML 写法：<code>{item.key ? `{{camofy.${item.key}}}` : "{{camofy.变量名}}"}</code>。未绑定时，必填变量阻止发布。</small>
    </div>)}
  </section>;
}

function selectorKey(selector: VariableSelector): string { return JSON.stringify(selector); }

export function ProvideEditor({ value, candidates, onChange }: { value: VariableExport[]; candidates: ExportCandidate[]; onChange: (value: VariableExport[]) => void }) {
  const update = (index: number, change: Partial<VariableExport>) => onChange(value.map((item, i) => i === index ? { ...item, ...change } : item));
  const addCandidate = (candidate: ExportCandidate) => {
    const used = new Set(value.map((item) => item.key));
    let n = 1;
    while (used.has(`value_${n}`)) n += 1;
    onChange([...value, { key: `value_${n}`, label: candidate.label, type: candidate.type, selector: candidate.selector }]);
  };
  return <section className="capability-editor">
    <div className="section-heading"><div><h3>导出变量</h3><p className="muted">此 Profile 向身份提供的值，由你在 Camofy 中定义，上游无需配合。可让不同 Profile 使用同一个导出变量名，切换启用关系时沿用身份绑定。</p></div>
      <button type="button" onClick={() => onChange([...value, { key: "", label: "", type: "string", selector: { source: "literal", value: "" } }])}>添加固定值</button>
    </div>
    {candidates.length > 0 && <label>或从当前 YAML 提取
      <select aria-label="从 YAML 提取导出值" value="" onChange={(e) => { const candidate = candidates[Number(e.target.value)]; if (candidate) addCandidate(candidate); }}>
        <option value="">选择一个字段或名称…</option>
        {candidates.map((candidate, index) => <option key={`${candidate.label}-${index}`} value={index}>{candidate.label}</option>)}
      </select>
    </label>}
    {value.map((item, index) => <div className="variable-contract-row" key={index}>
      <label>变量名<input required pattern="(?:[A-Za-z0-9_]|-){1,64}" maxLength={64} value={item.key} onChange={(e) => update(index, { key: e.target.value })} placeholder="例如 main_group" /></label>
      <label>显示名称<input required maxLength={120} value={item.label} onChange={(e) => update(index, { label: e.target.value })} /></label>
      <label>类型<TypeSelect value={item.type} onChange={(type) => update(index, { type })} /></label>
      <label>值的来源<select aria-label="值的来源" value={item.selector.source} onChange={(e) => {
        const source = e.target.value;
        const named = candidates.find((candidate) => candidate.selector.source === "named");
        update(index, { selector: source === "literal" ? { source: "literal", value: "" } : source === "named" && named ? named.selector : { source: "pointer", path: "/" } });
      }}><option value="literal">直接填写</option><option value="named">从 YAML 中按名称提取</option><option value="pointer">从 YAML 路径提取</option></select></label>
      {item.selector.source === "pointer" && <label>JSON Pointer<input required value={item.selector.path} onChange={(e) => update(index, { selector: { ...item.selector, path: e.target.value } as VariableSelector })} placeholder="/字段/子字段" /></label>}
      {item.selector.source === "named" && <label>已选目标<select aria-label="已选目标" value={selectorKey(item.selector)} onChange={(e) => {
        const candidate = candidates.find((c) => selectorKey(c.selector) === e.target.value);
        if (candidate) update(index, { selector: candidate.selector, type: candidate.type });
      }}><option value={selectorKey(item.selector)}>{item.selector.section} / {String(item.selector.match_value)}</option>
        {candidates.filter((c) => c.selector.source === "named" && selectorKey(c.selector) !== selectorKey(item.selector)).map((c) => <option key={selectorKey(c.selector)} value={selectorKey(c.selector)}>{c.label}</option>)}
      </select></label>}
      {item.selector.source === "literal" && <label>提供的值<LiteralInput key={`${index}-${item.type}`} type={item.type} value={item.selector.value} onChange={(v) => update(index, { selector: { source: "literal", value: v } })} /></label>}
      <button type="button" className="danger" onClick={() => onChange(value.filter((_, i) => i !== index))}>移除</button>
    </div>)}
  </section>;
}

export function BindingPicker({ value, type, providers, aliases, onChange }: {
  value?: VariableBinding; type: VariableType; providers: Resource[]; aliases: Record<string, IdentityValue>; onChange: (value?: VariableBinding) => void;
}) {
  const exportedKeys = new Map<string, VariableExport[]>();
  for (const profile of providers) for (const provided of profile.data.provides ?? []) {
    const entries = exportedKeys.get(provided.key) ?? [];
    entries.push(provided);
    exportedKeys.set(provided.key, entries);
  }
  const compatibleExports = [...exportedKeys].filter(([, entries]) =>
    entries.every((provided) => provided.type === type || (type === "string" && provided.type === "outbound")));
  const compatibleAliases = Object.entries(aliases).filter(([, alias]) => alias.type === type || (type === "string" && alias.type === "outbound"));
  const selected = value?.source === "export" ? `export:${value.key}`
    : value?.source === "identity" ? `identity:${value.key}`
      : value?.source === "literal" ? "literal" : "unset";
  return <div className="variable-binding">
    <select aria-label="变量来源" value={selected} onChange={(e) => {
      const source = e.target.value;
      if (source === "literal") onChange({ source: "literal", value: type === "boolean" ? false : type === "integer" || type === "number" ? 0 : type === "list" ? [] : type === "object" ? {} : "" });
      else if (source.startsWith("export:")) {
        const key = source.slice("export:".length);
        if (compatibleExports.some(([candidate]) => candidate === key)) onChange({ source: "export", key });
      } else if (source.startsWith("identity:")) onChange({ source: "identity", key: source.slice(9) });
      else onChange(undefined);
    }}>
      <option value="unset">未绑定（使用声明的默认值）</option>
      <option value="literal">直接填写</option>
      {value?.source === "export" && !compatibleExports.some(([key]) => key === value.key) && <option value={selected}>当前不可用的导出变量 · {value.key}</option>}
      {compatibleExports.length > 0 && <optgroup label="已启用 Profile 导出的变量">{compatibleExports.map(([key, entries]) =>
        <option key={key} value={`export:${key}`}>{key}{entries.length > 1 ? " · 多个提供方，预览将报冲突" : ""}</option>)}</optgroup>}
      {value?.source === "identity" && !compatibleAliases.some(([key]) => key === value.key) && <option value={selected}>已失效的身份变量</option>}
      {compatibleAliases.length > 0 && <optgroup label="身份变量">{compatibleAliases.map(([key]) => <option key={key} value={`identity:${key}`}>{key}</option>)}</optgroup>}
    </select>
    {value?.source === "export" && <small className="variable-contract-hint">绑定变量 <code>{value.key}</code>；由当前身份中唯一启用的提供方赋值。</small>}
    {value?.source === "literal" && <LiteralInput key={type} type={type} value={value.value} onChange={(v) => onChange({ source: "literal", value: v })} />}
  </div>;
}

export function IdentityValuesEditor({ value, providers, onChange }: { value: Record<string, IdentityValue>; providers: Resource[]; onChange: (value: Record<string, IdentityValue>) => void }) {
  const entries = Object.entries(value);
  const replace = (oldKey: string, newKey: string, next: IdentityValue) => {
    const result = { ...value };
    delete result[oldKey];
    result[newKey] = next;
    onChange(result);
  };
  return <section className="capability-editor"><div className="section-heading"><div><h3>身份变量</h3><p className="muted">在这个身份内命名可复用的值。输入名与导出变量名可以不同，必须明确绑定；同名导出同时启用会报冲突。</p></div>
    <button type="button" onClick={() => { let i = 1; while (value[`value_${i}`]) i++; onChange({ ...value, [`value_${i}`]: { type: "string", binding: { source: "literal", value: "" } } }); }}>添加身份变量</button></div>
    {entries.map(([key, item]) => <div className="variable-contract-row" key={key}>
      <label>变量名<input required pattern="(?:[A-Za-z0-9_]|-){1,64}" maxLength={64} value={key} onChange={(e) => replace(key, e.target.value, item)} /></label>
      <label>类型<TypeSelect value={item.type} onChange={(type) => replace(key, key, { type, binding: { source: "literal", value: type === "boolean" ? false : type === "integer" || type === "number" ? 0 : type === "list" ? [] : type === "object" ? {} : "" } })} /></label>
      <BindingPicker type={item.type} value={item.binding} providers={providers} aliases={Object.fromEntries(entries.filter(([other]) => other !== key))} onChange={(binding) => { if (binding) replace(key, key, { ...item, binding }); }} />
      <button type="button" className="danger" onClick={() => { const result = { ...value }; delete result[key]; onChange(result); }}>移除</button>
    </div>)}
  </section>;
}
