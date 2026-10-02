import { useCallback, useEffect, useRef, useState } from "react"
import { registerSessionCleanup } from "@/app/providers"
import { stopScanner } from "@/wasm/client"
import { cameraErrorMessage } from "./media"

const LOCATION_MAX_AGE_MS = 10_000

type FacingMode = "user" | "environment"
type LocationWaiter = {
  resolve: (position: GeolocationPosition) => void
  reject: (error: Error) => void
  signal: AbortSignal
  timer: number
  onAbort: () => void
}

export function useCaptureSession() {
  const videoRef = useRef<HTMLVideoElement>(null)
  const streamRef = useRef<MediaStream | null>(null)
  const watchIdRef = useRef<number | null>(null)
  const latestPositionRef = useRef<GeolocationPosition | null>(null)
  const locationErrorRef = useRef<string | null>(null)
  const waitersRef = useRef<Set<LocationWaiter>>(new Set())
  const controllerRef = useRef<AbortController | null>(null)
  const [facingMode, setFacingMode] = useState<FacingMode | null>(null)
  const [location, setLocation] = useState<GeolocationPosition | null>(null)
  const [locationStatus, setLocationStatus] = useState<
    "idle" | "waiting" | "ready" | "error"
  >("idle")
  const [locationError, setLocationError] = useState<string | null>(null)

  const finishWaiter = useCallback((waiter: LocationWaiter) => {
    window.clearTimeout(waiter.timer)
    waiter.signal.removeEventListener("abort", waiter.onAbort)
    waitersRef.current.delete(waiter)
  }, [])

  const stopCamera = useCallback(() => {
    const stream = streamRef.current
    streamRef.current = null
    if (stream) {
      for (const track of stream.getTracks()) track.stop()
    }
    if (videoRef.current && videoRef.current.srcObject === stream)
      videoRef.current.srcObject = null
    setFacingMode(null)
  }, [])

  const stopLocation = useCallback(() => {
    if (watchIdRef.current !== null && navigator.geolocation) {
      navigator.geolocation.clearWatch(watchIdRef.current)
    }
    watchIdRef.current = null
    latestPositionRef.current = null
    locationErrorRef.current = null
    setLocation(null)
    setLocationStatus("idle")
    setLocationError(null)
  }, [])

  const stopSession = useCallback(() => {
    controllerRef.current?.abort()
    controllerRef.current = null
    stopCamera()
    stopLocation()
    stopScanner()
    for (const waiter of waitersRef.current) {
      finishWaiter(waiter)
      waiter.reject(new DOMException("采集已取消", "AbortError"))
    }
  }, [finishWaiter, stopCamera, stopLocation])

  useEffect(() => {
    const unregister = registerSessionCleanup(stopSession)
    return () => {
      unregister()
      stopSession()
    }
  }, [stopSession])

  const beginSession = useCallback(() => {
    stopSession()
    const controller = new AbortController()
    controllerRef.current = controller
    setLocationError(null)
    setLocation(null)
    setLocationStatus("idle")
    return controller
  }, [stopSession])

  const startCamera = useCallback(
    async (mode: FacingMode, signal: AbortSignal) => {
      stopCamera()
      if (!navigator.mediaDevices?.getUserMedia) {
        const message =
          "当前浏览器不支持相机采集，请使用支持相机的安全连接后重试"
        throw new Error(message)
      }

      let stream: MediaStream
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          audio: false,
          video: {
            facingMode: { ideal: mode },
            width: { ideal: 1280, max: 1920 },
            height: { ideal: 720, max: 1920 },
          },
        })
      } catch (error) {
        if (signal.aborted) throw new DOMException("采集已取消", "AbortError")
        const message = cameraErrorMessage(error)
        throw new Error(message)
      }

      if (signal.aborted) {
        for (const track of stream.getTracks()) track.stop()
        throw new DOMException("采集已取消", "AbortError")
      }
      const video = videoRef.current
      if (!video) {
        for (const track of stream.getTracks()) track.stop()
        const message = "相机预览尚未准备好，请重试"
        throw new Error(message)
      }

      streamRef.current = stream
      video.srcObject = stream
      video.muted = true
      video.playsInline = true
      try {
        await video.play()
        if (
          video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA ||
          video.videoWidth < 1
        ) {
          await new Promise<void>((resolve, reject) => {
            let timer = 0
            const cleanup = () => {
              window.clearTimeout(timer)
              video.removeEventListener("loadeddata", ready)
              video.removeEventListener("error", failed)
              signal.removeEventListener("abort", cancelled)
            }
            const ready = () => {
              if (video.videoWidth < 1) return
              cleanup()
              resolve()
            }
            const failed = () => {
              cleanup()
              reject(new Error("无法读取相机画面"))
            }
            const cancelled = () => {
              cleanup()
              reject(new DOMException("采集已取消", "AbortError"))
            }
            timer = window.setTimeout(() => {
              cleanup()
              reject(new Error("相机未能启动，请检查设备后重试"))
            }, 10_000)
            video.addEventListener("loadeddata", ready)
            video.addEventListener("error", failed)
            signal.addEventListener("abort", cancelled, { once: true })
            if (signal.aborted) cancelled()
            else if (
              video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA &&
              video.videoWidth > 0
            )
              ready()
          })
        }
        if (signal.aborted) throw new DOMException("采集已取消", "AbortError")
        setFacingMode(mode)
      } catch (error) {
        for (const track of stream.getTracks()) track.stop()
        if (streamRef.current === stream) streamRef.current = null
        if (video.srcObject === stream) video.srcObject = null
        if (signal.aborted) throw new DOMException("采集已取消", "AbortError")
        const message = cameraErrorMessage(error)
        throw new Error(message)
      }
    },
    [stopCamera],
  )

  const startLocation = useCallback(() => {
    stopLocation()
    locationErrorRef.current = null
    setLocationError(null)
    if (!navigator.geolocation) {
      const message = "此设备无法提供定位，请检查定位权限或设备设置"
      locationErrorRef.current = message
      setLocationStatus("error")
      setLocationError(message)
      return
    }

    setLocationStatus("waiting")
    try {
      watchIdRef.current = navigator.geolocation.watchPosition(
        (position) => {
          latestPositionRef.current = position
          locationErrorRef.current = null
          setLocation(position)
          setLocationStatus("ready")
          setLocationError(null)
          for (const waiter of waitersRef.current) {
            if (
              Date.now() - position.timestamp <= LOCATION_MAX_AGE_MS &&
              Date.now() >= position.timestamp
            ) {
              finishWaiter(waiter)
              waiter.resolve(position)
            }
          }
        },
        (error) => {
          const message =
            error.code === error.PERMISSION_DENIED
              ? "定位权限被拒绝，请在浏览器设置中允许定位后重试"
              : error.code === error.POSITION_UNAVAILABLE
                ? "定位不可用，请检查设备定位服务后重试"
                : "定位超时，请重新获取定位"
          locationErrorRef.current = message
          setLocationStatus("error")
          setLocationError(message)
          for (const waiter of waitersRef.current) {
            finishWaiter(waiter)
            waiter.reject(new Error(message))
          }
        },
        { enableHighAccuracy: true, maximumAge: 0, timeout: 10_000 },
      )
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "定位启动失败，请重试"
      locationErrorRef.current = message
      setLocationStatus("error")
      setLocationError(message)
    }
  }, [finishWaiter, stopLocation])

  const waitForLocation = useCallback(
    (signal: AbortSignal): Promise<GeolocationPosition> => {
      const lastPosition = latestPositionRef.current
      if (
        lastPosition &&
        Date.now() - lastPosition.timestamp <= LOCATION_MAX_AGE_MS &&
        Date.now() >= lastPosition.timestamp
      ) {
        return Promise.resolve(lastPosition)
      }
      if (signal.aborted)
        return Promise.reject(new DOMException("采集已取消", "AbortError"))
      if (locationErrorRef.current)
        return Promise.reject(new Error(locationErrorRef.current))

      return new Promise((resolve, reject) => {
        const waiter = {} as LocationWaiter
        waiter.resolve = resolve
        waiter.reject = reject
        waiter.signal = signal
        waiter.onAbort = () => {
          finishWaiter(waiter)
          reject(new DOMException("采集已取消", "AbortError"))
        }
        waiter.timer = window.setTimeout(() => {
          finishWaiter(waiter)
          reject(new Error("定位等待超时，请重新获取定位"))
        }, 10_000)
        waitersRef.current.add(waiter)
        signal.addEventListener("abort", waiter.onAbort, { once: true })
        if (signal.aborted) {
          waiter.onAbort()
        } else {
          const newestPosition = latestPositionRef.current
          if (
            newestPosition &&
            Date.now() - newestPosition.timestamp <= LOCATION_MAX_AGE_MS &&
            Date.now() >= newestPosition.timestamp
          ) {
            finishWaiter(waiter)
            resolve(newestPosition)
          } else if (locationErrorRef.current) {
            finishWaiter(waiter)
            reject(new Error(locationErrorRef.current))
          }
        }
      })
    },
    [finishWaiter],
  )

  const currentPosition = useCallback(() => latestPositionRef.current, [])

  return {
    videoRef,
    facingMode,
    location,
    locationStatus,
    locationError,
    beginSession,
    stopSession,
    stopCamera,
    startCamera,
    startLocation,
    waitForLocation,
    currentPosition,
  }
}
