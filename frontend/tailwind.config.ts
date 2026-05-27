import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        surface: {
          DEFAULT: "#ffffff",
          alt: "#f5f5f7",
          card: "#ffffff",
          hover: "#ebebed",
        },
        sidebar: {
          DEFAULT: "rgba(255, 255, 255, 0.72)",
          hover: "rgba(0, 0, 0, 0.05)",
          active: "rgba(0, 122, 255, 0.12)",
          border: "rgba(0, 0, 0, 0.08)",
        },
        accent: {
          DEFAULT: "#007aff",
          light: "#4da6ff",
          dim: "#0062cc",
        },
        text: {
          primary: "#1d1d1f",
          secondary: "#6e6e73",
          muted: "#aeaeb2",
        },
        border: {
          DEFAULT: "#d2d2d7",
          light: "#e5e5ea",
        },
        table: {
          row: "#ffffff",
          alt: "#f9f9fb",
          selected: "rgba(0, 122, 255, 0.12)",
          selectedBorder: "rgba(0, 122, 255, 0.3)",
        },
      },
    },
  },
  plugins: [],
} satisfies Config;
