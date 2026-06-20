// M2 fixture: minimal Touying deck with an overlay (`#pause`) and a speaker note.
//
// Expected physical pages (presentation order):
//   page 0: slide 1, overlay 0  (with speaker note)
//   page 1: slide 1, overlay 1  (after #pause)
//   page 2: slide 2, overlay 0
//
// Compiling this fetches `@preview/touying` over the network on first run.
#import "@preview/touying:0.7.4": *
#import themes.simple: *

#show: simple-theme.with(aspect-ratio: "4-3")

== Overlay slide

#speaker-note[Note for the first slide.]

First bullet
#pause
Second bullet

== Plain slide

Just one step.
