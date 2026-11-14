import { mount } from "svelte";
import Internal from "../placeholder/Internal.svelte";

const target = document.getElementById("app");
if (target) mount(Internal, { target });
