import { memo, useEffect, useRef, useState } from "react"
import * as L from "leaflet"
import "leaflet/dist/leaflet.css"

type TeacherLocationMapProps = {
  latitude: string
  longitude: string
  radius: string
  onCoordinatesChange: (latitude: number, longitude: number) => void
}

const INITIAL_CENTER: L.LatLngTuple = [35.8617, 104.1954]
const INITIAL_ZOOM = 4

const locationIcon = L.divIcon({
  className: "teacher-location-map__marker",
  html: '<span aria-hidden="true"></span>',
  iconSize: [22, 22],
  iconAnchor: [11, 11],
})

function coordinatesFromFields(
  latitude: string,
  longitude: string,
): L.LatLngTuple | null {
  if (!latitude.trim() || !longitude.trim()) return null

  const latitudeValue = Number(latitude)
  const longitudeValue = Number(longitude)
  if (
    !Number.isFinite(latitudeValue) ||
    latitudeValue < -90 ||
    latitudeValue > 90 ||
    !Number.isFinite(longitudeValue) ||
    longitudeValue < -180 ||
    longitudeValue > 180
  ) {
    return null
  }

  return [latitudeValue, longitudeValue]
}


export const TeacherLocationMap = memo(function TeacherLocationMap({
  latitude,
  longitude,
  radius,
  onCoordinatesChange,
}: TeacherLocationMapProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const mapRef = useRef<L.Map | null>(null)
  const markerRef = useRef<L.Marker | null>(null)
  const circleRef = useRef<L.Circle | null>(null)
  const onCoordinatesChangeRef = useRef(onCoordinatesChange)
  const [mapError, setMapError] = useState<string | null>(null)

  onCoordinatesChangeRef.current = onCoordinatesChange

  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    let map: L.Map | null = null
    let tileLayer: L.TileLayer | null = null
    let marker: L.Marker | null = null
    let circle: L.Circle | null = null
    let resizeObserver: ResizeObserver | null = null
    let frameId = 0
    let tileLoadFailed = false

    try {
      map = L.map(container, {
        center: INITIAL_CENTER,
        zoom: INITIAL_ZOOM,
        zoomControl: true,
        scrollWheelZoom: false,
        keyboard: true,
      })

      map.on("click", (event: L.LeafletMouseEvent) => {
        const { lat, lng } = event.latlng.wrap()
        onCoordinatesChangeRef.current(lat, lng)
      })

      marker = L.marker(INITIAL_CENTER, {
        icon: locationIcon,
        draggable: true,
        keyboard: true,
        title: "考勤地点，可拖动调整",
        riseOnHover: true,
      })
      marker.on("dragend", () => {
        const point = marker?.getLatLng().wrap()
        if (point) onCoordinatesChangeRef.current(point.lat, point.lng)
      })

      circle = L.circle(INITIAL_CENTER, {
        radius: 0,
        color: "#1d4ed8",
        weight: 2,
        fillColor: "#3b82f6",
        fillOpacity: 0.14,
        interactive: false,
      })

      tileLayer = L.tileLayer(
        "https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png",
        {
          maxZoom: 19,
          attribution:
            '&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noreferrer">OpenStreetMap</a> contributors',
        },
      )
        .on("loading", () => {
          tileLoadFailed = false
        })
        .on("tileerror", () => {
          tileLoadFailed = true
          setMapError("地图底图加载失败，请检查网络，或手动填写 WGS84 坐标。")
        })
        .on("load", () => {
          if (!tileLoadFailed) setMapError(null)
        })
        .addTo(map)

      mapRef.current = map
      markerRef.current = marker
      circleRef.current = circle
      frameId = window.requestAnimationFrame(() => {
        map?.invalidateSize({ pan: false })
      })
      if (typeof ResizeObserver !== "undefined") {
        resizeObserver = new ResizeObserver(() => {
          map?.invalidateSize({ pan: false })
        })
        resizeObserver.observe(container)
      }
    } catch {
      setMapError("地图加载失败，仍可手动填写 WGS84 坐标。")
    }

    return () => {
      resizeObserver?.disconnect()
      if (frameId) window.cancelAnimationFrame(frameId)
      tileLayer?.off()
      marker?.off()
      map?.off()
      map?.remove()
      mapRef.current = null
      markerRef.current = null
      circleRef.current = null
    }
  }, [])

  useEffect(() => {
    const map = mapRef.current
    const marker = markerRef.current
    const circle = circleRef.current
    if (!map || !marker || !circle) return

    const coordinates = coordinatesFromFields(latitude, longitude)
    if (!coordinates) {
      marker.remove()
      circle.remove()
      return
    }

    const point = L.latLng(coordinates[0], coordinates[1])
    const firstPoint = !map.hasLayer(marker)
    marker.setLatLng(point)
    if (firstPoint) {
      marker.addTo(map)
      map.setView(point, 16, { animate: false })
    }
    const radiusMeters = Number(radius)
    if (
      !radius.trim() ||
      !Number.isFinite(radiusMeters) ||
      radiusMeters <= 0
    ) {
      circle.remove()
    } else {
      circle.setLatLng(point).setRadius(radiusMeters)
      if (!map.hasLayer(circle)) circle.addTo(map)
    }

    if (!map.getBounds().contains(point)) map.panTo(point, { animate: false })
  }, [latitude, longitude, radius])

  return (
    <section className="grid gap-2 sm:col-span-2">
      <div>
        <h4 className="font-medium">地图选点</h4>
        <p
          id="stage-location-map-help"
          className="text-sm text-muted-foreground"
        >
          点击地图放置标记，拖动标记微调位置；圆圈表示当前定位半径。坐标使用 WGS84。
        </p>
      </div>
      <div
        ref={containerRef}
        className="teacher-location-map__surface h-72 w-full overflow-hidden rounded-md border bg-muted sm:h-80"
        role="region"
        aria-label="考勤地点地图，点击选择坐标"
        aria-describedby="stage-location-map-help"
      />
      {mapError && (
        <p role="status" aria-live="polite" className="text-sm text-destructive">
          {mapError}
        </p>
      )}
      <style>{`
        .teacher-location-map__surface {
          font-family: inherit;
        }
        .teacher-location-map__marker {
          background: transparent;
          border: 0;
        }
        .teacher-location-map__marker > span {
          box-sizing: border-box;
          display: block;
          width: 20px;
          height: 20px;
          border: 3px solid white;
          border-radius: 9999px;
          background: #1456f0;
          box-shadow: 0 1px 5px rgb(10 10 10 / 35%);
        }
      `}</style>
    </section>
  )
})
