/** Fence asynchronous provider receipts against turn/session replacement. */
export const providerEventStillCurrent = (receivingVoice, currentVoice, receivingSessionSequence, currentSessionSequence, locallyFencedSessionSequence) => receivingSessionSequence === currentSessionSequence
    && receivingVoice === currentVoice
    && !(receivingSessionSequence > 0
        && locallyFencedSessionSequence === receivingSessionSequence);
