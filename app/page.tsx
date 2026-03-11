"use client";

import { FormEvent, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function HomePage() {
  const [name, setName] = useState("");
  const [greeting, setGreeting] = useState("");
  const [isLoading, setIsLoading] = useState(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setIsLoading(true);

    try {
      const response = await invoke<string>("greet", { name });
      setGreeting(response);
    } finally {
      setIsLoading(false);
    }
  }

  return (
    <main className="shell">
      <section className="panel">
        <p className="eyebrow">Tauri v2 + Next.js</p>
        <h1>Static-export Next frontend for your desktop app.</h1>
        <p className="lede">
          This project now uses Next.js for the UI while keeping the existing
          Rust command bridge through Tauri.
        </p>

        <form className="greetForm" onSubmit={handleSubmit}>
          <label className="field">
            <span>Name</span>
            <input
              value={name}
              onChange={(event) => setName(event.currentTarget.value)}
              placeholder="Enter a name"
            />
          </label>
          <button type="submit" disabled={isLoading}>
            {isLoading ? "Greeting..." : "Greet via Rust"}
          </button>
        </form>

        <p className="response">{greeting || "Your Tauri response will appear here."}</p>
      </section>
    </main>
  );
}
