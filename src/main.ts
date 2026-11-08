import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/forms.css";
import { mount } from "svelte";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) throw new Error("#app が見つかりません");

export default mount(App, { target });
