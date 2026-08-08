;; Any component import is a forbidden host capability. This fixture deliberately
;; imports an instance and requires no implementation because rejection precedes
;; linking and renderer-world validation.
(component
  (type $capability (instance
    (type $open-type (func))
    (export "open" (func (type $open-type)))))
  (import "malicious:host/capability" (instance $capability)))
