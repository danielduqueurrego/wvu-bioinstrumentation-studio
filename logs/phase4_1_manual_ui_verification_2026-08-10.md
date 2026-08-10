# Phase 4.1 manual UI verification

Status: accepted user-assisted verification at 100% Windows display scaling.

| Viewport | Windows scaling | Navigation | Plot groups | Overflow/clipping | Result | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| 900 x 650 | 100% | passed | passed | none observed | passed | Required narrow layout check completed |
| 1024 x 768 | 100% | passed | passed | none observed | passed | Required medium layout check completed |
| 1366 x 768 | 100% | passed | passed | none observed | passed | Required desktop layout check completed |
| 1920 x 1080 | 100% | passed | passed | none observed | passed | Required wide layout check completed |
| maximized | 100% | passed | passed | none observed | passed | User confirmed live grouping and controls work |

The user confirmed EMG regrouping/hide-show, BP pressure grouping, pulse-ox grouping,
responsive navigation, and reachable Start, Stop, and Marker controls. Windows 125% and
150% scaling were not tested and remain pending rather than inferred.

Required live checks: EMG four-to-two-to-one groups; BP pressure grouping; pulse-ox TX/RX
grouping; trace hide/show without interrupting recording; reachable Start, Stop, and Marker;
responsive navigation without a Validation route.
