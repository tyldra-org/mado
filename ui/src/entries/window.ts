import MainWindow from "../placeholder/MainWindow.svelte";
import { start } from "./mount";

// The label is ui-<WindowId> (spec 01 §4.1).
start(MainWindow, (store) => ({ store, windowId: store.transport.label().replace(/^ui-/, "") }));
