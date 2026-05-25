import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        surface: {
          DEFAULT: "#1a1b2e",
          alt: "#222336",
          card: "#2a2b40",
          hover: "#323350",
        },
        accent: {
          DEFAULT: "#0d9488",
          light: "#14b8a6",
          dim: "#0f766e",
        },
        text: {
          primary: "#e8e8ed",
          secondary: "#a1a1b5",
          muted: "#6b6b80",
        },
      },
    },
  },
  plugins: [],
} satisfies Config;
