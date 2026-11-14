import { mount } from "svelte";
import DevPlatform from "../placeholder/DevPlatform.svelte";

const target = document.getElementById("app");
if (target) mount(DevPlatform, { target });
