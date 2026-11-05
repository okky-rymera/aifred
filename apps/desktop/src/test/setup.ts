import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";
import { clearMocks } from "@tauri-apps/api/mocks";

// The test browser (jsdom) has no scrolling.
Element.prototype.scrollIntoView = vi.fn();

afterEach(() => {
  cleanup();
  clearMocks();
});

// jsdom has no scroll on elements either.
Element.prototype.scrollTo = vi.fn() as unknown as typeof Element.prototype.scrollTo;
