function mapIntoArray(children, array, escapedPrefix, nameSoFar, callback) {
    var type = typeof children;
    ("undefined" === type || "boolean" === type) && (children = null);
    var invokeCallback = !1;
    if (null === children) invokeCallback = !0;
    else switch(type){
        case "string":
        case "number":
            invokeCallback = !0;
            break;
        case "object":
            switch(children.$$typeof){
                case REACT_ELEMENT_TYPE:
                case REACT_PORTAL_TYPE:
                    invokeCallback = !0;
            }
    }
    if (invokeCallback) {
        var _child = children, mappedChild = callback(_child), childKey = "" === nameSoFar ? "." + getElementKey(_child, 0) : nameSoFar;
        if (Array.isArray(mappedChild)) {
            var escapedChildKey = "";
            null != childKey && (escapedChildKey = escapeUserProvidedKey(childKey) + "/"), mapIntoArray(mappedChild, array, escapedChildKey, "", function(c) {
                return c;
            });
        } else null != mappedChild && (isValidElement(mappedChild) && (mappedChild = cloneAndReplaceKey(mappedChild, escapedPrefix + (mappedChild.key && (!_child || _child.key !== mappedChild.key) ? escapeUserProvidedKey("" + mappedChild.key) + "/" : "") + childKey)), array.push(mappedChild));
        return 1;
    }
    var subtreeCount = 0, nextNamePrefix = "" === nameSoFar ? "." : nameSoFar + SUBSEPARATOR;
    if (Array.isArray(children)) for(var i = 0; i < children.length; i++)subtreeCount += mapIntoArray(child, array, escapedPrefix, nextNamePrefix + getElementKey(child = children[i], i), callback);
    else {
        var iteratorFn = getIteratorFn(children);
        if ("function" == typeof iteratorFn) {
            var child, step, iterableChildren = children;
            iteratorFn === iterableChildren.entries && (didWarnAboutMaps || warn("Using Maps as children is not supported. Use an array of keyed ReactElements instead."), didWarnAboutMaps = !0);
            for(var iterator = iteratorFn.call(iterableChildren), ii = 0; !(step = iterator.next()).done;)subtreeCount += mapIntoArray(child, array, escapedPrefix, nextNamePrefix + getElementKey(child = step.value, ii++), callback);
        } else if ("object" === type) {
            var childrenString = "" + children;
            throw Error("Objects are not valid as a React child (found: " + ("[object Object]" === childrenString ? "object with keys {" + Object.keys(children).join(", ") + "}" : childrenString) + "). If you meant to render a collection of children, use an array instead.");
        }
    }
    return subtreeCount;
}
