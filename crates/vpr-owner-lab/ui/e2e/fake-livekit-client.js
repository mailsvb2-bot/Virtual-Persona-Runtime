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
  let testDevicesReady = false;
  const deviceChangeListeners = [];

  class FakeTrack {
    constructor(kind, deviceId = "expressive-mic") {
      trackSequence += 1;
      this.id = `expressive-track-${trackSequence}`;
      this.kind = kind;
      this.deviceId = deviceId;
    }
    stop() {}
    getSettings() {
      return this.kind === "audio" ? { deviceId: this.deviceId } : {};
    }
  }

  class FakeMediaStream {
    constructor(tracks = []) {
      this.tracks = [...tracks];
    }
    getTracks() { return [...this.tracks]; }
    getAudioTracks() { return this.tracks.filter((track) => track.kind === "audio"); }
    addTrack(track) { this.tracks.push(track); }
  }

  class FakeAnalyser {
    constructor() { this.fftSize = 256; }
    connect() {}
    disconnect() {}
    getFloatTimeDomainData(samples) {
      samples.fill(window.__vprExpressiveRemoteSpeech === true ? 0.12 : 0.0005);
    }
  }

  class FakeGain {
    constructor() { this.gain = { value: 1 }; }
    connect() {}
    disconnect() {}
  }

  class FakeAudioContext {
    constructor(options) {
      this.sampleRate = options?.sampleRate ?? 48000;
      this.destination = {};
      this.audioWorklet = { addModule: async () => undefined };
    }
    async resume() {}
    async close() {}
    createMediaStreamSource() {
      return { connect() {}, disconnect() {} };
    }
    createAnalyser() { return new FakeAnalyser(); }
    createGain() { return new FakeGain(); }
  }

  class FakeAudioWorkletNode {
    constructor() {
      this.port = { onmessage: null };
    }
    connect() {
      const samples = new Float32Array(4800);
      samples.fill(0.2);
      queueMicrotask(() => this.port.onmessage?.({ data: samples.buffer }));
    }
    disconnect() {}
  }

  const mediaDevices = {
    async enumerateDevices() {
      if (!testDevicesReady) return [];
      return [{
        deviceId: "expressive-mic",
        kind: "audioinput",
        label: "Expressive test microphone",
        groupId: "g1",
        toJSON: () => ({}),
      }];
    },
    async getUserMedia() {
      return new FakeMediaStream([new FakeTrack("audio", "expressive-mic")]);
    },
    addEventListener(type, listener) {
      if (type !== "devicechange") return;
      deviceChangeListeners.push(() => {
        if (typeof listener === "function") listener(new Event("devicechange"));
        else listener.handleEvent(new Event("devicechange"));
      });
    },
  };

  class FakeRemoteTrack {
    constructor(kind) {
      this.kind = kind;
      this.mediaStreamTrack = new FakeTrack(kind);
      this.statsPoll = 0;
    }
    attach(element) {
      element.style.width = "4096px";
      element.style.height = "4096px";
      if (window.__vprForceMediaElementAvSync === true) {
        const currentTime = this.kind === "audio" ? 10 : 10.06;
        Object.defineProperty(element, "currentTime", {
          configurable: true,
          get: () => currentTime,
        });
      }
      return element;
    }
    async getRTCStatsReport() {
      this.statsPoll += 1;
      const timestamp = this.kind === "audio" ? 1000 : 1060;
      const entries = [
        [
          `${this.kind}-codec`,
          { mimeType: this.kind === "audio" ? "audio/opus" : "video/VP8" },
        ],
        [
          `${this.kind}-inbound-stale`,
          {
            type: "inbound-rtp",
            kind: this.kind,
            codecId: `${this.kind}-codec`,
            packetsReceived: 7,
            ...(window.__vprForceMediaElementAvSync === true || this.statsPoll < 3
              ? {}
              : { estimatedPlayoutTimestamp: timestamp + 4000 }),
          },
        ],
        [
          `${this.kind}-inbound`,
          {
            type: "inbound-rtp",
            kind: this.kind,
            codecId: `${this.kind}-codec`,
            packetsReceived: 20 + this.statsPoll * 5,
            ...(window.__vprForceMediaElementAvSync === true || this.statsPoll < 3
              ? {}
              : { estimatedPlayoutTimestamp: timestamp }),
          },
        ],
      ];
      if (this.kind === "video") {
        entries.push(
          ["video-rtx-codec", { mimeType: "video/rtx" }],
          [
            "video-rtx-inbound",
            {
              type: "inbound-rtp",
              kind: "video",
              codecId: "video-rtx-codec",
              packetsReceived: 100,
              ...(window.__vprForceMediaElementAvSync === true
                ? {}
                : { estimatedPlayoutTimestamp: 9999 }),
            },
          ],
        );
      }
      return new Map(entries);
    }
  }

  class FakeRoom {
    constructor() {
      this.handlers = new Map();
      this.audioTrack = null;
      this.videoTrack = null;
      this.localParticipant = {
        sendText: async (text, options) => {
          if (options.topic === "did.interrupt" && window.__vprExpressiveFailNextInterrupt === true) {
            window.__vprExpressiveFailNextInterrupt = false;
            throw new Error("SIMULATED_PROVIDER_STOP_FAILED");
          }
          commands.push({ topic: options.topic, text });
          if (options.topic === "did.speak") window.__vprExpressiveRemoteSpeech = true;
          if (options.topic === "did.interrupt") window.__vprExpressiveRemoteSpeech = false;
        },
      };
      window.__vprExpressiveDisconnect = () => this.emit(roomEvents.Disconnected);
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
      window.__vprExpressiveGenerationDone = () => {
        window.__vprExpressiveRemoteSpeech = false;
        // Unknown provider events must be classified into bounded groups, never
        // captured as raw subject/content or promoted into playback completion.
        for (const subject of [
          "chat/answer", "stream-video/started", "stream-video/done",
          "chat/new-private-event", "stream-video/unknown", "tool-call/new-private-event",
          "completely-private-event",
        ]) {
          this.emit(
            roomEvents.DataReceived,
            new TextEncoder().encode(JSON.stringify({ subject, content: "PRIVATE_MEDIA_DIAGNOSTIC_MUST_NOT_LEAK" })),
          );
        }
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
      testDevicesReady = true;
      queueMicrotask(() => deviceChangeListeners.forEach((listener) => listener()));
      this.videoTrack = new FakeRemoteTrack("video");
      this.audioTrack = new FakeRemoteTrack("audio");
      this.emit(roomEvents.TrackSubscribed, this.videoTrack);
      this.emit(roomEvents.TrackSubscribed, this.audioTrack);
    }
    async disconnect() {
      window.__vprExpressiveRoomDisconnectCount += 1;
    }
  }

  window.__vprExpressiveRoomDisconnectCount = 0;
  window.__vprLiveKitCommands = commands;
  window.__vprExpressiveRemoteSpeech = false;
  const installMediaRuntime = () => {
    if (window.__vprMediaRuntime) return;
    window.__vprMediaRuntime = {
      mediaDevices,
      MediaStream: FakeMediaStream,
      AudioContext: FakeAudioContext,
      AudioWorkletNode: FakeAudioWorkletNode,
      setSrcObject(element, value) {
        element.__vprTestSrcObject = value;
      },
      requestVideoFrame(_element, callback) {
        queueMicrotask(callback);
        return 1;
      },
    };
  };

  installMediaRuntime();
  window.LivekitClient = { Room: FakeRoom, RoomEvent: roomEvents };
})();
