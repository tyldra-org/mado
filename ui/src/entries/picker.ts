import { mount } from "svelte";
import Picker from "../placeholder/Picker.svelte";

const target = document.getElementById("app");
if (target) mount(Picker, { target });
