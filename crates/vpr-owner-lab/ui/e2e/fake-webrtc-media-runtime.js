(() => {
  let remoteSpeech = false;
  let remoteTrackPublished = false;
  let publishRemoteAudioTrack = null;
  let beginSyntheticPlayback = null;
  let trackSequence = 0;
  let playbackSequence = 0;
  let activePeer = null;
  let providerDataChannel = null;
  const requestedMicrophones = [];
  const interruptPayloads = [];
  const deviceChangeListeners = [];
  let testDevicesReady = false;

  window.__vprRequestedMicrophones = requestedMicrophones;
  window.__vprInterruptPayloads = interruptPayloads;

  class FakeTrack {
    constructor(kind = "audio", deviceId = "builtin-mic") {
      trackSequence += 1;
      this.id = `fake-track-${trackSequence}`;
      this.kind = kind;
      this.deviceId = deviceId;
    }
    stop() {}
    getSettings() { return { deviceId: this.deviceId }; }
  }

  class FakeMediaStream {
    constructor(tracks = []) { this.tracks = [...tracks]; }
    getTracks() { return [...this.tracks]; }
    getAudioTracks() { return this.tracks.filter((track) => track.kind === "audio"); }
    addTrack(track) { this.tracks.push(track); }
  }

  class FakeAnalyser {
    constructor() { this.fftSize = 256; }
    connect() {}
    disconnect() {}
    getFloatTimeDomainData(samples) {
      samples.fill(remoteSpeech ? 0.12 : 0.0005);
    }
  }

  class FakeGain {
    constructor() { this.gain = { value: 1 }; }
    connect() {}
    disconnect() {}
  }

  class FakeAudioContext {
    constructor(options) {
      this.sampleRate = options?.sampleRate ?? 48_000;
      this.destination = {};
      this.audioWorklet = { addModule: async () => undefined };
    }
    async resume() {}
    async close() {}
    createMediaStreamSource() { return { connect() {}, disconnect() {} }; }
    createAnalyser() { return new FakeAnalyser(); }
    createGain() { return new FakeGain(); }
  }

  class FakeAudioWorkletNode {
    constructor() {
      this.port = { onmessage: null };
    }
    connect() {
      const samples = new Float32Array(4_800);
      samples.fill(0.2);
      queueMicrotask(() => this.port.onmessage?.({ data: samples.buffer }));
    }
    disconnect() {
      // The provider begins playback after the microphone turn closes. This is owned
      // entirely by the fake transport; production HTTP completion does not drive it.
      window.setTimeout(() => beginSyntheticPlayback?.(), 0);
    }
  }

  class FakePeerConnection {
    constructor() {
      this.connectionState = "new";
      this.ontrack = null;
      this.onconnectionstatechange = null;
      this.onicecandidate = null;
      activePeer = this;
      publishRemoteAudioTrack = () => {
        if (remoteTrackPublished) return;
        remoteTrackPublished = true;
        this.ontrack?.({ track: new FakeTrack("audio") });
      };
    }

    createDataChannel(label) {
      const channel = {
        label,
        readyState: "open",
        onopen: null,
        onclose: null,
        onmessage: null,
        send(payload) {
          if (!payload.includes("stream/interrupt")) {
            throw new Error("UNEXPECTED_WEBRTC_CLIENT_COMMAND");
          }
          interruptPayloads.push(payload);
          remoteSpeech = false;
          queueMicrotask(() => channel.onmessage?.({ data: "stream/done:{}" }));
        },
        close() {
          channel.readyState = "closed";
          channel.onclose?.();
        },
      };
      providerDataChannel = channel;
      queueMicrotask(() => channel.onopen?.());
      return channel;
    }

    async setRemoteDescription() {}
    async createAnswer() {
      return { type: "answer", sdp: "v=0 voice-browser-answer" };
    }
    async setLocalDescription() {
      testDevicesReady = true;
      queueMicrotask(() => deviceChangeListeners.forEach((listener) => listener()));
      window.setTimeout(() => {
        this.connectionState = "connected";
        this.onconnectionstatechange?.();
      }, 0);
    }
    async getStats() {
      return new Map([
        ["audio", { type: "inbound-rtp", kind: "audio", packetsReceived: 20, estimatedPlayoutTimestamp: 1_000 }],
        ["video", { type: "inbound-rtp", kind: "video", packetsReceived: 20, estimatedPlayoutTimestamp: 1_060 }],
      ]);
    }
    close() { this.connectionState = "closed"; }
  }

  const mediaDevices = {
    async enumerateDevices() {
      if (!testDevicesReady) return [];
      return [
        { deviceId: "builtin-mic", kind: "audioinput", label: "Встроенный микрофон", groupId: "g1", toJSON: () => ({}) },
        { deviceId: "headset-mic", kind: "audioinput", label: "Микрофон гарнитуры", groupId: "g2", toJSON: () => ({}) },
      ];
    },
    async getUserMedia(constraints) {
      const audio = typeof constraints.audio === "object" && constraints.audio !== null
        ? constraints.audio
        : {};
      const requested = typeof audio.deviceId === "object"
        && audio.deviceId !== null
        && "exact" in audio.deviceId
        ? String(audio.deviceId.exact)
        : "builtin-mic";
      requestedMicrophones.push(requested);
      return new FakeMediaStream([new FakeTrack("audio", requested)]);
    },
    addEventListener(type, listener) {
      if (type !== "devicechange") return;
      deviceChangeListeners.push(() => {
        if (typeof listener === "function") listener(new Event("devicechange"));
        else listener.handleEvent(new Event("devicechange"));
      });
    },
  };

  beginSyntheticPlayback = () => {
    publishRemoteAudioTrack?.();
    remoteSpeech = true;
    playbackSequence += 1;
    providerDataChannel?.onmessage?.({
      data: `stream/started:${JSON.stringify({ metadata: { videoId: `video-${playbackSequence}` } })}`,
    });
  };

  window.__vprSetPeerConnectionState = (state) => {
    if (!activePeer) throw new Error("NO_ACTIVE_PEER");
    activePeer.connectionState = state;
    activePeer.onconnectionstatechange?.();
  };
  window.__vprCreateMicStream = (deviceId) =>
    new FakeMediaStream([new FakeTrack("audio", deviceId)]);
  window.__vprMediaRuntime = {
    mediaDevices,
    AudioContext: FakeAudioContext,
    AudioWorkletNode: FakeAudioWorkletNode,
    MediaStream: FakeMediaStream,
    RTCPeerConnection: FakePeerConnection,
    setSrcObject(element, value) {
      element.__vprTestSrcObject = value;
    },
  };
})();
