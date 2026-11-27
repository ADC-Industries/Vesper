import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { APP_FULL_NAME } from "./lib/config";

document.title = APP_FULL_NAME;

const app = mount(App, { target: document.getElementById("app")! });

export default app;
