(() => {
  const commands = [];
  const roomEvents = {
    TrackSubscribed: "track-subscribed",
    TrackUnsubscribed: "track-unsubscribed",
    DataReceived: "data-received",
    Reconnecting: "reconnecting",
    Reconnected: "reconnected",
    Disconnected: "disconnected",
  };
  let trackSequence = 0;

  class FakeRemoteTrack {
    constructor(kind) {
      this.kind = kind;
      trackSequence += 1;
      this.mediaStreamTrack = {
        id: `expressive-provider-track-${trackSequence}`,
        kind,
        stop() {},
        getSettings() { return kind === "audio" ? { deviceId: "expressive-mic" } : {}; },
      };
    }
    attach(element) {
      element.style.width = "4096px";
      element.style.height = "4096px";
      return element;
    }
    async getRTCStatsReport() {
      const timestamp = this.kind === "audio" ? 1000 : 1060;
      return new Map([[
        `${this.kind}-inbound`,
        {
          type: "inbound-rtp",
          kind: this.kind,
          packetsReceived: 20,
          estimatedPlayoutTimestamp: timestamp,
        },
      ]]);
    }
  }

  class FakeRoom {
    constructor() {
      this.handlers = new Map();
      this.audioTrack = null;
      this.videoTrack = null;
      this.localParticipant = {
        sendText: async (text, options) => {
          commands.push({ topic: options.topic, text });
          if (options.topic === "did.speak") window.__vprExpressiveRemoteSpeech = true;
          if (options.topic === "did.interrupt") window.__vprExpressiveRemoteSpeech = false;
        },
      };
      window.__vprExpressiveDisconnect = () => {
        this.emit(roomEvents.Disconnected);
      };
      window.__vprExpressiveLoseAudio = () => {
        const track = this.audioTrack;
        if (!track) return;
        this.audioTrack = null;
        this.emit(roomEvents.TrackUnsubscribed, track);
      };
      window.__vprExpressiveRestoreAudio = () => {
        if (this.audioTrack) return;
        const track = new FakeRemoteTrack("audio");
        this.audioTrack = track;
        this.emit(roomEvents.TrackSubscribed, track);
      };
      window.__vprExpressiveLoseVideo = () => {
        const track = this.videoTrack;
        if (!track) return;
        this.videoTrack = null;
        this.emit(roomEvents.TrackUnsubscribed, track);
      };
      window.__vprExpressiveRestoreVideo = () => {
        if (this.videoTrack) return;
        const track = new FakeRemoteTrack("video");
        this.videoTrack = track;
        this.emit(roomEvents.TrackSubscribed, track);
      };
      window.__vprExpressivePlaybackDone = () => {
        window.__vprExpressiveRemoteSpeech = false;
        this.emit(
          roomEvents.DataReceived,
          new TextEncoder().encode(JSON.stringify({ subject: "stream-video/done" })),
        );
      };
    }

    on(event, handler) {
      const handlers = this.handlers.get(event) || [];
      handlers.push(handler);
      this.handlers.set(event, handlers);
      return this;
    }

    emit(event, ...args) {
      for (const handler of this.handlers.get(event) || []) handler(...args);
    }

    async connect() {
      await new Promise((resolve) => window.setTimeout(resolve, 0));
      window.__vprExpressiveProviderConnected?.();
      this.videoTrack = new FakeRemoteTrack("video");
      this.audioTrack = new FakeRemoteTrack("audio");
      this.emit(roomEvents.TrackSubscribed, this.videoTrack);
      this.emit(roomEvents.TrackSubscribed, this.audioTrack);
    }

    async disconnect() {}
  }

  window.__vprLiveKitCommands = commands;
  window.__vprExpressiveRemoteSpeech = false;
  window.LivekitClient = { Room: FakeRoom, RoomEvent: roomEvents };
})();
