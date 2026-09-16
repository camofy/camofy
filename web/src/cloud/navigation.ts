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
    sub: "管理上游订阅、拉取出口与自动更新。",
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
    name: "拉取代理",
    sub: "为订阅配置可靠的网络出口。",
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
export const resourcePath = (r: Resource) => `/${sectionOf(r)}/${r.id}`;
