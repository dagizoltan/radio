# Verify why the mock signal gradient might be frozen or incorrectly sized
# Wait, if `.vu-track` is `display: flex; flex-direction: column-reverse;`,
# The `.vu-bar` with `height: 58%` will grow from the bottom!
# BUT `.vu-bar` has `background: linear-gradient(to top, green, yellow, red)`.
# If `height` is 58%, the linear-gradient will STRETCH to fit the 58% height!
# So the top of the 58% bar will be RED!
# That means it won't look like a real meter where it's green at the bottom and red at the top!
# To fix this, we should NOT change the height of `.vu-bar`.
# We should change the `transform: translateY` or use a mask!
