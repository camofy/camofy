import type { Resource } from "./model";
export type Section =
  "identities" | "subscriptions" | "profiles" | "proxies" | "devices";
export const sections: {
  key: Section;
  name: string;
  sub: string;
  icon: string;
}[] = [
  {
    key: "identities",
    name: "身份",
    sub: "组合配置，分发到每一端。",
    icon: "layers",
  },
  {
    key: "subscriptions",
    name: "订阅源",
    sub: "管理上游订阅与自动更新，拉取出口由平台统一管理。",
    icon: "radio",
  },
  {
    key: "profiles",
    name: "配置 Profile",
    sub: "把节点、规则和运行参数组织成可复用的配置。",
    icon: "code",
  },
  {
    key: "proxies",
    name: "订阅出口",
    sub: "管理全平台订阅的统一出口，不影响客户端的代理策略。",
    icon: "route",
  },
  {
    key: "devices",
    name: "设备",
    sub: "查看设备上报与配置应用情况。",
    icon: "monitor",
  },
];
export function sectionOf(r: Resource): Section {
  return r.kind === "profile"
    ? r.data.type === "source"
      ? "subscriptions"
      : "profiles"
    : r.kind === "bundle"
      ? "identities"
      : r.kind === "proxy"
        ? "proxies"
        : "devices";
}
/** A source's own subscription link: an identity that is shown on its source page. */
export const managedSource = (r: Resource) =>
  r.kind === "bundle" ? r.data.managed_source : undefined;
export const listed = (r: Resource) => !managedSource(r);
export const displayName = (r: Resource) =>
  managedSource(r) ? `订阅源 · ${r.data.name}` : r.data.name;
export const resourcePath = (r: Resource) => {
  const source = managedSource(r);
  return source ? `/subscriptions/${source}` : `/${sectionOf(r)}/${r.id}`;
};
