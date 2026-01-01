# Chat

## Username

The frontend maintains a chat username:

- Stored in `localStorage` key `chatusername`
- Sent with each chat message as `username`

The UI exposes a username editor in the chat panel.

## Mentions (ping)

Mentions are plain-text `@username` tokens inside a message:

- While typing `@`, the chat composer shows an autocomplete list of usernames that have spoken in the current session.
- If a message mentions your current username, the message is highlighted and a subtle bottom-screen notification is shown.

## Replies

Chat supports replies as first-class metadata on messages:

- Outgoing payload includes `reply_to_id` and `reply_to_username`
- The UI shows the replied-to username and a short preview of the replied-to message (if available in the local message history).

## Frequency tokens

Chat messages can include inline frequency tokens:

- Format: `[FREQ:<hz>:<mode>]`
- Example: `[FREQ:7074000:USB]`

Clicking a rendered token tunes the receiver to that frequency/mode.

## URL query parameters

NovaSDR supports URL parameters for share links:

- `modulation`: `USB | LSB | CW | AM | SAM | FM | WBFM`
- `frequency`: interpreted heuristically as:
  - **MHz** when `< 1000` (example: `7.074`)
  - **kHz** when `< 1000000` (example: `7074.0`)
  - **Hz** otherwise (example: `7074000`)


