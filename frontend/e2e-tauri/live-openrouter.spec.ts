describe("Live OpenRouter assistant", () => {
  it("completes a native assistant turn and persists the API call", async () => {
    const result = await browser.execute(async () => {
      const event = await window.api.assistantSend({
        message: "Say hello in one short sentence.",
        apiKey: "configured-in-native-process",
        selectedTrackPaths: [],
        tracks: [],
        albums: [],
        autonomous: false,
      });
      const current = await window.api.getCurrentSession();
      const session = current
        ? await window.api.getSession(current.sessionId)
        : null;
      return { event, session };
    });

    if (result.event.type !== "message") {
      throw new Error(`assistant returned ${JSON.stringify(result.event)}`);
    }
    expect(result.event.message.trim().length).toBeGreaterThan(0);
    expect(result.session?.apiCallCount).toBeGreaterThanOrEqual(1);
  });
});
