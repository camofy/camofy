import type {
  CapabilityBinding,
  OutboundExport,
  OutboundInput,
  Resource,
} from "./model";

export function OutboundPicker({
  label,
  value,
  providers,
  onChange,
  allowDefault = true,
}: {
  label: string;
  value?: CapabilityBinding;
  providers: Resource[];
  onChange: (value: CapabilityBinding | undefined) => void;
  allowDefault?: boolean;
}) {
  const exports = providers.flatMap((p) =>
    (p.data._exports ?? p.data.exports ?? []).map((e) => ({ profile: p, item: e })),
  );
  const selected = !value || value.source === "default"
    ? "default"
    : value.source === "literal"
      ? "literal"
      : `${value.profile_id}:${value.key}`;
  return (
    <div className="capability-picker">
      <label>
        {label}
        <select
          value={selected}
          onChange={(e) => {
            const selected = e.target.value;
            if (selected === "default") onChange(undefined);
            else if (selected === "literal") onChange({ source: "literal", value: "" });
            else {
              const found = exports.find(
                (x) => `${x.profile.id}:${x.item.key}` === selected,
              );
              if (found)
                onChange({ source: "export", profile_id: found.profile.id, key: found.item.key });
            }
          }}
        >
          {allowDefault && <option value="default">使用身份默认出口</option>}
          {!allowDefault && <option value="default">自动选择唯一默认出口</option>}
          {exports.map(({ profile, item }) => (
            <option key={`${profile.id}:${item.key}`} value={`${profile.id}:${item.key}`}>
              {profile.data.name} / {item.label} → {item.target}
            </option>
          ))}
          <option value="literal">手动指定最终节点或分组名称…</option>
        </select>
      </label>
      {value?.source === "literal" && (
        <label>
          最终节点或分组名称
          <input
            value={value.value}
            maxLength={200}
            placeholder="与合并后的配置中的名称完全一致"
            onChange={(e) => onChange({ source: "literal", value: e.target.value })}
          />
        </label>
      )}
    </div>
  );
}

export function ExportEditor({
  value,
  automatic,
  onChange,
}: {
  value: OutboundExport[];
  automatic?: OutboundExport[];
  onChange: (value: OutboundExport[]) => void;
}) {
  return (
    <section className="capability-editor">
      <h3>提供给其他 Profile 的出口</h3>
      <p className="muted">这里映射现有节点或分组。订阅内容刷新时，映射由 Camofy 保留并校验。</p>
      {automatic?.filter((e) => !value.some((v) => v.key === e.key)).map((e) => (
        <p className="muted" key={e.key}>自动发现：{e.label} → {e.target}</p>
      ))}
      {value.map((entry, index) => (
        <div className="capability-row" key={index}>
          <label>标识<input required value={entry.key} placeholder="default" onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, key: e.target.value } : x))} /></label>
          <label>显示名称<input required value={entry.label} placeholder="默认出口" onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, label: e.target.value } : x))} /></label>
          <label>类型<select value={entry.kind} onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, kind: e.target.value as OutboundExport["kind"] } : x))}><option value="group">代理组</option><option value="proxy">节点</option></select></label>
          <label>现有名称<input required value={entry.target} placeholder="Proxies" onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, target: e.target.value } : x))} /></label>
          <button type="button" onClick={() => onChange(value.filter((_, n) => n !== index))}>移除</button>
        </div>
      ))}
      <button type="button" className="quiet" onClick={() => onChange([...value, { key: "", label: "", kind: "group", target: "" }])}>添加出口</button>
    </section>
  );
}

export function InputEditor({ value, onChange }: {
  value: OutboundInput[];
  onChange: (value: OutboundInput[]) => void;
}) {
  return (
    <section className="capability-editor">
      <h3>需要身份提供的出口</h3>
      <p className="muted">为当前 YAML 中的节点指定前置代理。实际值在每个身份中分别绑定。</p>
      {value.map((entry, index) => (
        <div className="capability-row" key={index}>
          <label>标识<input required value={entry.key} placeholder="upstream" onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, key: e.target.value } : x))} /></label>
          <label>显示名称<input required value={entry.label} placeholder="前置代理" onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, label: e.target.value } : x))} /></label>
          <label>节点所在字段<select value={entry.section} onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, section: e.target.value as OutboundInput["section"] } : x))}><option value="proxies">proxies</option><option value="prepend-proxies">prepend-proxies</option><option value="append-proxies">append-proxies</option></select></label>
          <label>节点名称<input required value={entry.name} placeholder="在 YAML 中的 name" onChange={(e) => onChange(value.map((x, n) => n === index ? { ...x, name: e.target.value } : x))} /></label>
          <button type="button" onClick={() => onChange(value.filter((_, n) => n !== index))}>移除</button>
        </div>
      ))}
      <button type="button" className="quiet" onClick={() => onChange([...value, { key: "", label: "", kind: "outbound", section: "prepend-proxies", name: "", field: "dialer-proxy" }])}>添加输入</button>
    </section>
  );
}
