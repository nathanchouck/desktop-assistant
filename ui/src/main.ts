import { mount } from "svelte";
import App from "./App.svelte";
import { commands } from "./lib/bindings";
import "katex/dist/katex.min.css";
import "./styles/tokens.css";
import "./styles/base.css";

const target = document.getElementById("app");
if (!target) throw new Error("#app element missing from index.html");

// Switch to the translucent surface when the OS blur is active.
void commands.appInfo().then((info) => {
  if (info.glass) document.documentElement.dataset.glass = "on";
});

export default mount(App, { target });
