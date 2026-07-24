/// Whether the overlay passes input to its target or accepts interaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InteractionMode {
    /// Mouse and keyboard input pass through to the target window.
    #[default]
    PassThrough,
    /// The overlay receives mouse and keyboard input.
    Interactive,
}

/// A mouse button reported by the overlay window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

/// A physical key transition identified by its Windows virtual-key code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyState {
    Pressed,
    Released,
}

/// Input and focus events collected from the overlay window.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    MouseMoved {
        x: f32,
        y: f32,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
        x: f32,
        y: f32,
    },
    MouseWheel {
        delta: f32,
        x: f32,
        y: f32,
    },
    Key {
        virtual_key: u16,
        state: KeyState,
    },
    Text(char),
    Focused(bool),
    CloseRequested,
}
