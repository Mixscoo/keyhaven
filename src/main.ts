import { mount } from "svelte";
import "./lib/design/tokens.css";
import "./lib/design/global.css";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Could not find #app mount target");
}

const app = mount(App, { target });

export default app;
