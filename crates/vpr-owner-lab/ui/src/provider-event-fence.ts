/** Fence asynchronous provider receipts against turn/session replacement. */
export const providerEventStillCurrent = (
  receivingVoice: object | null,
  currentVoice: object | null,
  receivingSessionSequence: number,
  currentSessionSequence: number,
  locallyFencedSessionSequence: number,
): boolean =>
  receivingSessionSequence === currentSessionSequence
  && receivingVoice === currentVoice
  && !(receivingSessionSequence > 0
    && locallyFencedSessionSequence === receivingSessionSequence);
