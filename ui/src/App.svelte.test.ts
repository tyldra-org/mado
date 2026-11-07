import { render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import App from "./App.svelte";

test("shows the app name as the page heading", () => {
  render(App);
  expect(screen.getByRole("heading", { level: 1, name: "Mado" })).toBeTruthy();
});

test("renders inside a main landmark", () => {
  render(App);
  expect(screen.getByRole("main")).toBeTruthy();
});
