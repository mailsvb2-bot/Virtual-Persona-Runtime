export type MediaRuntimeOverrides = {
  mediaDevices?: Pick<MediaDevices, "enumerateDevices" | "getUserMedia" | "addEventListener">;
  AudioContext?: unknown;
  AudioWorkletNode?: unknown;
  MediaStream?: unknown;
  RTCPeerConnection?: unknown;
  liveKitSdk?: unknown;
  setSrcObject?: (element: HTMLMediaElement, value: MediaProvider | null) => void;
  requestVideoFrame?: (element: HTMLVideoElement, callback: () => void) => number;
};

declare global {
  interface Window {
    __vprMediaRuntime?: MediaRuntimeOverrides;
  }
}

export const mediaRuntime = (): MediaRuntimeOverrides | undefined => window.__vprMediaRuntime;

export const runtimeFetch = window.fetch.bind(window);

export const runtimeMediaDevices = (): MediaDevices | MediaRuntimeOverrides["mediaDevices"] =>
  mediaRuntime()?.mediaDevices ?? navigator.mediaDevices;

export const createRuntimeAudioContext = (options?: AudioContextOptions): AudioContext => {
  const Constructor = (mediaRuntime()?.AudioContext ?? window.AudioContext) as typeof AudioContext;
  return new Constructor(options);
};

export const createRuntimeAudioWorkletNode = (
  context: BaseAudioContext,
  name: string,
): AudioWorkletNode => {
  const Constructor = (mediaRuntime()?.AudioWorkletNode ?? window.AudioWorkletNode) as typeof AudioWorkletNode;
  return new Constructor(context, name);
};

export const createRuntimeMediaStream = (tracks?: MediaStreamTrack[]): MediaStream => {
  const Constructor = (mediaRuntime()?.MediaStream ?? window.MediaStream) as typeof MediaStream;
  return tracks ? new Constructor(tracks) : new Constructor();
};

export const createRuntimePeerConnection = (configuration?: RTCConfiguration): RTCPeerConnection => {
  const Constructor = (mediaRuntime()?.RTCPeerConnection ?? window.RTCPeerConnection) as typeof RTCPeerConnection;
  return new Constructor(configuration);
};

export const setMediaSrcObject = (element: HTMLMediaElement, value: MediaProvider | null): void => {
  const setter = mediaRuntime()?.setSrcObject;
  if (setter) setter(element, value);
  else element.srcObject = value;
};

export const requestVideoFrame = (element: HTMLVideoElement, callback: () => void): boolean => {
  const request = mediaRuntime()?.requestVideoFrame;
  if (request) {
    request(element, callback);
    return true;
  }
  const nativeRequest = (element as HTMLVideoElement & {
    requestVideoFrameCallback?: (callback: () => void) => number;
  }).requestVideoFrameCallback;
  if (typeof nativeRequest !== "function") return false;
  nativeRequest.call(element, callback);
  return true;
};
